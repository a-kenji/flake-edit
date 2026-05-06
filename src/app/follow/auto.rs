//! Auto-deduplication of nested follows.
//!
//! Three structs thread through the pipeline:
//!
//! - `AnalysisCtx`: read-only inputs (graph, config, bounds).
//! - `FollowPlan`: mutable accumulator threaded through collection.
//! - `AppliedPlan`: outcomes of the apply step, consumed by `render_summary`.

use std::collections::{HashMap, HashSet};

use crate::change::{Change, ChangeId};
use crate::config::FollowConfig;
use crate::edit::{FlakeEdit, InputMap};
use crate::follows::{
    AttrPath, Edge, EdgeOrigin, FollowsGraph, Segment, is_follows_reference_to_parent,
};
use crate::input::Range;
use crate::lock::{FlakeLock, NestedInput};
use crate::validate;

use super::super::commands::{self, CommandError, Result};
use super::super::editor::Editor;
use super::super::state::AppState;

/// Entry point for `flake-edit follow` on a single in-memory flake.
pub fn run(editor: &Editor, flake_edit: &mut FlakeEdit, state: &AppState) -> Result<()> {
    run_impl(editor, flake_edit, state, false)
}

/// Run auto-follow against in-memory text.
///
/// This is exposed for benchmark and library callers that need the same
/// planner and in-memory edit path as `flake-edit follow` without file I/O.
#[doc(hidden)]
pub fn run_in_memory(
    flake_text: &str,
    lock_text: &str,
    follow_config: &FollowConfig,
) -> Result<Option<String>> {
    let mut flake_edit = FlakeEdit::from_text(flake_text).map_err(CommandError::FlakeEdit)?;
    let lock = FlakeLock::read_from_str(lock_text).map_err(CommandError::FlakeEdit)?;
    let nested_inputs = lock.nested_inputs();
    if nested_inputs.is_empty() {
        return Ok(None);
    }

    let inputs = flake_edit.list().clone();
    if inputs.is_empty() {
        return Ok(None);
    }

    let top_level_inputs: HashSet<String> = inputs.keys().cloned().collect();
    let graph = FollowsGraph::from_flake(&inputs, &lock);
    let Some(plan) = build_plan(
        flake_text,
        &nested_inputs,
        top_level_inputs,
        &inputs,
        &graph,
        follow_config,
    ) else {
        return Ok(None);
    };

    let applied = apply_plan_text(flake_text, Some(&lock), &inputs, &plan)?;
    Ok((applied.current_text != flake_text).then_some(applied.current_text))
}

/// Entry point for batch mode (`flake-edit follow [PATHS...]`).
///
/// Each file is processed independently with its own [`Editor`] and
/// [`AppState`]; processing continues past per-file failures. Any
/// failures are bundled into a single [`CommandError::Batch`].
pub fn run_batch(
    paths: &[std::path::PathBuf],
    transitive: Option<usize>,
    depth: Option<usize>,
    args: &crate::cli::CliArgs,
) -> Result<()> {
    use std::path::PathBuf;

    let mut errors: Vec<(PathBuf, CommandError)> = Vec::new();

    for flake_path in paths {
        let lock_path = flake_path
            .parent()
            .map(|p| p.join("flake.lock"))
            .unwrap_or_else(|| PathBuf::from("flake.lock"));

        let editor = match Editor::from_path(flake_path.clone()) {
            Ok(e) => e,
            Err(e) => {
                errors.push((flake_path.clone(), e.into()));
                continue;
            }
        };

        let mut flake_edit = match editor.create_flake_edit() {
            Ok(fe) => fe,
            Err(e) => {
                errors.push((flake_path.clone(), e.into()));
                continue;
            }
        };

        let mut state = match AppState::new(
            editor.text(),
            flake_path.clone(),
            args.config().map(PathBuf::from),
        ) {
            Ok(s) => s
                .with_diff(args.diff())
                .with_no_lock(args.no_lock())
                .with_interactive(false)
                .with_lock_file(Some(lock_path))
                .with_no_cache(args.no_cache())
                .with_cache_path(args.cache().map(PathBuf::from)),
            Err(e) => {
                errors.push((flake_path.clone(), e.into()));
                continue;
            }
        };

        if let Some(min) = transitive {
            state.config.follow.transitive_min = min;
        }
        if let Some(max) = depth {
            state.config.follow.max_depth = max;
        }

        if let Err(e) = run_impl(&editor, &mut flake_edit, &state, true) {
            errors.push((flake_path.clone(), e));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(CommandError::Batch { failures: errors })
    }
}

/// Inputs threaded through analysis.
///
/// `top_level_inputs` is owned because [`run_impl`] extends it with names
/// minted by [`emit_direct_promotions`] before the later passes consult it
/// (see the pass ordering in [`run_impl`]). The other fields are derived
/// once from the on-disk text and the lockfile and stay put.
struct AnalysisCtx<'a> {
    nested_inputs: &'a [NestedInput],
    /// Top-level input names. Initialized from the parsed `flake.nix` and
    /// extended in-place between [`emit_direct_promotions`] and the
    /// passes that read it ([`collect_transitive_groups`] and
    /// [`collect_direct_candidates`]).
    top_level_inputs: HashSet<String>,
    /// Full input map. Cycle detection needs URLs.
    inputs: &'a crate::edit::InputMap,
    /// Lock-augmented graph: union of edges declared in `flake.nix` and edges
    /// the lockfile resolved. Used for cycle detection and stale-edge queries
    /// that need the fully-resolved view.
    graph: &'a FollowsGraph,
    /// Edges already written in the `flake.nix` source. Distinct from `graph`
    /// because a follows existing only in the lockfile (declared by an
    /// upstream flake) is still a candidate to write into the user's
    /// `flake.nix`.
    existing_follows: &'a HashSet<AttrPath>,
    follow_config: &'a FollowConfig,
    /// Maximum depth of follows declarations to write. `1` (default) writes
    /// depth-1 only. `2` or higher also writes grandchild and deeper.
    max_depth: usize,
    transitive_min: usize,
}

/// Mutable plan accumulated across the collection functions.
///
/// The four output buckets (`to_follow`, `to_unfollow`, `toplevel_follows`,
/// `toplevel_adds`) feed [`apply_plan`]. `seen_nested` is a dedup guard
/// threaded through [`collect_direct_candidates`],
/// [`collect_transitive_groups`], [`collect_direct_groups`],
/// [`emit_direct_promotions`], and [`emit_transitive_promotions`] to prevent
/// scheduling the same nested path twice
/// when both a direct-name match and a transitive group claim it.
/// It is dedup state, not applied output, so [`FollowPlan::has_pending`]
/// excludes it.
#[derive(Default)]
struct FollowPlan {
    to_follow: Vec<(AttrPath, AttrPath)>,
    to_unfollow: Vec<AttrPath>,
    toplevel_follows: Vec<(AttrPath, AttrPath)>,
    toplevel_adds: Vec<(String, String)>,
    seen_nested: HashSet<AttrPath>,
}

impl FollowPlan {
    /// True if at least one applicable change was scheduled. `seen_nested`
    /// is dedup state, not pending output, so it is excluded.
    fn has_pending(&self) -> bool {
        !self.to_follow.is_empty()
            || !self.to_unfollow.is_empty()
            || !self.toplevel_follows.is_empty()
            || !self.toplevel_adds.is_empty()
    }
}

/// What [`apply_plan`] committed to the working text.
#[derive(Default)]
struct AppliedPlan {
    /// Working text after every successful change. Equal to the original
    /// when no per-step change applied.
    current_text: String,
    /// `(source_path, target)` follows that were successfully applied,
    /// for the success summary.
    applied_follows: Vec<(AttrPath, AttrPath)>,
    /// Stale follows declarations that were removed.
    unfollowed: Vec<AttrPath>,
    /// Validation warnings observed across speculative applications, in
    /// arrival order. The caller deduplicates for display.
    warnings: Vec<validate::ValidationError>,
}

fn run_impl(
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
    quiet: bool,
) -> Result<()> {
    let Some(ctx) = commands::load_follow_context(flake_edit, state)? else {
        if !quiet {
            println!("Nothing to deduplicate.");
        }
        return Ok(());
    };

    // Two graphs serve two questions. `graph` (lock-augmented) answers
    // "would proposing edge X create a cycle, and which existing edges are
    // stale?". That needs the full resolved view. `existing_follows`
    // (declared-only) answers "is this nested input already followed in
    // flake.nix?". Emission must not be suppressed for an edge that the
    // lock resolved but the source never declared.
    let graph = match commands::load_flake_lock(state) {
        Ok(lock) => FollowsGraph::from_flake(&ctx.inputs, &lock),
        Err(_) => FollowsGraph::from_declared(&ctx.inputs),
    };

    let Some(plan) = build_plan(
        &editor.text(),
        &ctx.nested_inputs,
        ctx.top_level_inputs.clone(),
        &ctx.inputs,
        &graph,
        &state.config.follow,
    ) else {
        if !quiet {
            println!("All inputs are already deduplicated.");
        }
        return Ok(());
    };

    let applied = apply_plan(editor, state, &ctx.inputs, &plan)?;
    render_summary(editor, state, &applied, quiet)
}

fn build_plan(
    source_text: &str,
    nested_inputs: &[NestedInput],
    top_level_inputs: HashSet<String>,
    inputs: &InputMap,
    graph: &FollowsGraph,
    follow_config: &FollowConfig,
) -> Option<FollowPlan> {
    // Filter the merged graph by `EdgeOrigin::Declared` rather than rebuilding
    // a separate declared-only graph. The declared subset is already in
    // `graph`. Use [`FollowsGraph::declared_sources`] (not
    // [`FollowsGraph::declared_edges`]) so nulled `follows = ""`
    // declarations also count as already-handled and the
    // auto-deduplicator does not silently retarget them.
    let existing_follows: HashSet<AttrPath> = graph.declared_sources();

    let transitive_min = follow_config.transitive_min();
    let max_depth = follow_config.max_depth.max(1);

    // `to_unfollow` carries two removal classes: stale declared edges
    // (source absent from the lockfile) and depth-N edges that the
    // lockfile already routes via upstream propagation. The
    // [`FollowsGraph::lock_routes_to`] call passes `Some(edge)` so the
    // edge under test is excluded; without that the user's own
    // declaration counts as the route and every declared edge would
    // qualify.
    //
    // Seeding runs against the original `graph`: the post-removal clone
    // built below would chicken-and-egg this loop.
    let mut to_unfollow: Vec<AttrPath> = graph
        .stale_edges()
        .into_iter()
        .map(|e| e.source.clone())
        .collect();
    // Nulled twin of [`FollowsGraph::stale_edges`]: target-less follows
    // skip [`FollowsGraph::declared_edges`], so seed them separately.
    to_unfollow.extend(graph.stale_nulled_sources().into_iter().cloned());
    let stale_set: HashSet<AttrPath> = to_unfollow.iter().cloned().collect();
    for edge in graph.declared_edges() {
        if stale_set.contains(&edge.source) {
            continue;
        }
        // Depth-1 has no upstream parent to propagate from.
        if edge.source.len() < 3 {
            continue;
        }
        if edge.source.len() > max_depth + 1 {
            continue;
        }
        if graph.lock_routes_to(&edge.source, &edge.follows, Some(edge), &[]) {
            tracing::debug!(
                "Marking redundant follow for removal: {} -> {} (covered by upstream propagation)",
                edge.source,
                edge.follows,
            );
            to_unfollow.push(edge.source.clone());
        }
    }
    to_unfollow.sort();
    to_unfollow.dedup();

    // Discovery must see the post-removal graph. Without this, an edge
    // marked for removal still shapes the cycle and routing checks
    // below and can suppress a candidate the apply phase would
    // otherwise unblock, forcing a second `flake-edit follow`
    // invocation to converge. The clone is a discovery-only artifact:
    // the apply phase still runs `Change::Remove` against the
    // unmodified `editor.text()`.
    let mut graph_for_discovery = graph.clone();
    graph_for_discovery.drop_edges_with_sources(&to_unfollow);

    let mut ax = AnalysisCtx {
        nested_inputs,
        top_level_inputs,
        inputs,
        graph: &graph_for_discovery,
        existing_follows: &existing_follows,
        follow_config,
        max_depth,
        transitive_min,
    };

    let mut plan = FollowPlan {
        to_unfollow,
        ..FollowPlan::default()
    };

    // Run the minter (`emit_direct_promotions`) first and extend
    // `ax.top_level_inputs` with its new names before the readers
    // (`emit_transitive_promotions`, `collect_direct_candidates`) consult
    // it. Otherwise the readers would need a second pipeline iteration to
    // see the minted names.
    if transitive_min > 0 {
        let direct_groups = collect_direct_groups(&ax, &plan);
        emit_direct_promotions(&ax, source_text, direct_groups, &mut plan);

        for (name, _) in &plan.toplevel_adds {
            ax.top_level_inputs.insert(name.clone());
        }

        let transitive_groups = collect_transitive_groups(&ax, &plan);
        emit_transitive_promotions(&ax, transitive_groups, &mut plan);
    }

    collect_direct_candidates(&ax, &mut plan);

    scrub_redundant(&graph_for_discovery, &mut plan);

    if !plan.has_pending() {
        return None;
    }

    Some(plan)
}

/// Depth-bounded path-shape filter shared by every collection function.
///
/// Path length encodes depth: `parent.nested` is depth 1 (length 2),
/// `parent.middle.grandchild` is depth 2 (length 3). The bound
/// `len() <= max_depth + 1` admits exactly the configured depth.
fn within_depth(path: &AttrPath, max_depth: usize) -> bool {
    path.len() >= 2 && path.len() <= max_depth + 1
}

/// True when an ancestor of `nested_path` already declares a follows
/// that contradicts the proposed `target`. An ancestor here is a path
/// formed by truncating one or more middle segments while preserving
/// the leading parent and the trailing name.
///
/// "Contradicts" means: the ancestor declares a different follows
/// target, or the ancestor declares `follows = ""` (nulled). In either
/// case the user has explicitly chosen a value for that subtree's
/// mapping of the trailing name, and a deeper proposal pointing
/// somewhere else would silently override that choice.
///
/// Ancestor declarations pointing at the same `target` do not count:
/// they reinforce, rather than override, the deeper proposal, so the
/// auto-deduplicator should still be free to emit the deeper edge.
///
/// For `[parent, mid1, ..., name]` of length `n >= 3`, checks every
/// candidate `[parent, mid1, ..., midk, name]` of length `2..n - 1`.
fn ancestor_overrides_subtree(
    nested_path: &AttrPath,
    target: &AttrPath,
    graph: &FollowsGraph,
) -> bool {
    if nested_path.len() < 3 {
        return false;
    }
    let segs = nested_path.segments();
    let last = nested_path.last();
    let nulled = graph.declared_nulled();
    for prefix_len in 1..nested_path.len() - 1 {
        let mut candidate = AttrPath::new(segs[0].clone());
        for seg in &segs[1..prefix_len] {
            candidate.push(seg.clone());
        }
        candidate.push(last.clone());
        if nulled.contains(&candidate) {
            return true;
        }
        for edge in graph.outgoing(&candidate) {
            if !matches!(edge.origin, EdgeOrigin::Declared { .. }) {
                continue;
            }
            if &edge.follows != target {
                return true;
            }
        }
    }
    false
}

fn collect_direct_candidates(ax: &AnalysisCtx<'_>, plan: &mut FollowPlan) {
    // Shallowest-first: depth-1 emissions must land in `plan.to_follow`
    // before depth-N candidates consult `extra_edges` below.
    let mut iter: Vec<&NestedInput> = ax.nested_inputs.iter().collect();
    iter.sort_by(|a, b| {
        a.path
            .len()
            .cmp(&b.path.len())
            .then_with(|| a.path.cmp(&b.path))
    });
    for nested in iter {
        if !within_depth(&nested.path, ax.max_depth) {
            continue;
        }
        let parent = nested.path.first().as_str();
        let nested_name = nested.path.last().as_str();
        let path_display = nested.path.to_string();

        if ax.follow_config.is_ignored(&path_display, nested_name) {
            tracing::debug!("Skipping {}: ignored by config", path_display);
            continue;
        }

        if ax.existing_follows.contains(&nested.path) {
            tracing::debug!("Skipping {}: already follows in flake.nix", path_display);
            continue;
        }

        let Some(target) = ax
            .top_level_inputs
            .iter()
            .find(|top| ax.follow_config.can_follow(nested_name, top))
        else {
            continue;
        };

        if let Some(target_input) = ax.inputs.get(target.as_str())
            && is_follows_reference_to_parent(target_input.url(), parent)
        {
            tracing::debug!(
                "Skipping {} -> {}: would create cycle (target follows {}/...)",
                path_display,
                target,
                parent,
            );
            continue;
        }

        // Top-level input names parse as a single-segment attr path. A
        // failure here would mean an invalid Nix identifier slipped in, so
        // skip rather than queue something the apply step would discard.
        let target_path = match AttrPath::parse(target) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("Skipping {path_display} -> {target}: invalid attr path: {e}");
                continue;
            }
        };

        if ancestor_overrides_subtree(&nested.path, &target_path, ax.graph) {
            tracing::debug!(
                "Skipping {} -> {}: ancestor declares a different follows for the same trailing name",
                path_display,
                target,
            );
            continue;
        }

        // Multi-hop / lockfile-only cycle detection. The URL-prefix check
        // above catches only the immediate-parent case. The merged graph
        // DFS catches the rest.
        let proposed = Edge {
            source: nested.path.clone(),
            follows: target_path.clone(),
            origin: EdgeOrigin::Declared {
                range: Range { start: 0, end: 0 },
            },
        };
        if ax.graph.would_create_cycle(&proposed) {
            tracing::debug!(
                "Skipping {} -> {}: would create cycle (multi-hop or lockfile-resolved)",
                path_display,
                target,
            );
            continue;
        }
        // Same-run depth-1 emissions can close a depth-N chain.
        let extra_edges: Vec<(AttrPath, AttrPath)> = if nested.path.len() >= 3 {
            plan.to_follow
                .iter()
                .map(|(src, tgt)| (src.clone(), tgt.clone()))
                .collect()
        } else {
            Vec::new()
        };
        if ax
            .graph
            .lock_routes_to(&nested.path, &target_path, None, &extra_edges)
        {
            tracing::debug!(
                "Skipping {} -> {}: lockfile already routes via upstream propagation",
                path_display,
                target,
            );
            continue;
        }

        plan.seen_nested.insert(nested.path.clone());
        plan.to_follow.push((nested.path.clone(), target_path));
    }
}

fn collect_transitive_groups(
    ax: &AnalysisCtx<'_>,
    plan: &FollowPlan,
) -> HashMap<String, HashMap<AttrPath, Vec<AttrPath>>> {
    let mut groups: HashMap<String, HashMap<AttrPath, Vec<AttrPath>>> = HashMap::new();

    for nested in ax.nested_inputs.iter() {
        if !within_depth(&nested.path, ax.max_depth) {
            continue;
        }
        let nested_name = nested.path.last().as_str();
        let parent = nested.path.first().as_str();
        let path_display = nested.path.to_string();

        if ax.follow_config.is_ignored(&path_display, nested_name) {
            continue;
        }
        if ax.existing_follows.contains(&nested.path) || plan.seen_nested.contains(&nested.path) {
            continue;
        }
        // Handled by [`collect_direct_candidates`].
        if ax
            .top_level_inputs
            .iter()
            .any(|top| ax.follow_config.can_follow(nested_name, top))
        {
            continue;
        }

        let Some(transitive_target) = nested.follows.as_ref() else {
            continue;
        };
        if ancestor_overrides_subtree(&nested.path, transitive_target, ax.graph) {
            continue;
        }
        // Only transitive follows (path with a parent segment).
        if transitive_target.len() < 2 {
            continue;
        }
        // Skip self-follow.
        if transitive_target.last().as_str() == nested_name {
            continue;
        }

        let top_level_name = ax
            .follow_config
            .resolve_alias(nested_name)
            .unwrap_or(nested_name)
            .to_string();
        if ax.top_level_inputs.contains(&top_level_name) {
            continue;
        }

        if let Some(target_input) = ax.inputs.get(transitive_target.first().as_str())
            && is_follows_reference_to_parent(target_input.url(), parent)
        {
            continue;
        }

        // Multi-hop / lockfile-only cycle detection, mirroring the
        // direct-candidate filter applied to transitive promotions.
        let proposed = Edge {
            source: nested.path.clone(),
            follows: transitive_target.clone(),
            origin: EdgeOrigin::Declared {
                range: Range { start: 0, end: 0 },
            },
        };
        if ax.graph.would_create_cycle(&proposed) {
            continue;
        }

        groups
            .entry(top_level_name)
            .or_default()
            .entry(transitive_target.clone())
            .or_default()
            .push(nested.path.clone());
    }

    groups
}

/// When several parents share the same dependency (e.g. `treefmt.nixpkgs`
/// and `treefmt-nix.nixpkgs`), one can be promoted to top-level and the
/// others follow it.
fn collect_direct_groups(
    ax: &AnalysisCtx<'_>,
    plan: &FollowPlan,
) -> HashMap<String, Vec<(AttrPath, Option<String>)>> {
    let mut groups: HashMap<String, Vec<(AttrPath, Option<String>)>> = HashMap::new();

    for nested in ax.nested_inputs.iter() {
        if !within_depth(&nested.path, ax.max_depth) {
            continue;
        }
        if nested.follows.is_some() {
            continue;
        }
        let nested_name = nested.path.last().as_str();
        let path_display = nested.path.to_string();

        if ax.follow_config.is_ignored(&path_display, nested_name) {
            continue;
        }
        if ax.existing_follows.contains(&nested.path) || plan.seen_nested.contains(&nested.path) {
            continue;
        }
        if ax
            .top_level_inputs
            .iter()
            .any(|top| ax.follow_config.can_follow(nested_name, top))
        {
            continue;
        }

        let canonical_name = ax
            .follow_config
            .resolve_alias(nested_name)
            .unwrap_or(nested_name)
            .to_string();
        if ax.top_level_inputs.contains(&canonical_name) {
            continue;
        }

        groups
            .entry(canonical_name)
            .or_default()
            .push((nested.path.clone(), nested.url.clone()));
    }

    groups
}

/// Turn transitive groups into top-level follows and back-fill
/// `plan.to_follow` with the per-nested follows hanging off them.
///
/// Must run after [`emit_direct_promotions`] so the top-level names it
/// minted are visible in `ax.top_level_inputs`.
fn emit_transitive_promotions(
    ax: &AnalysisCtx<'_>,
    transitive_groups: HashMap<String, HashMap<AttrPath, Vec<AttrPath>>>,
    plan: &mut FollowPlan,
) {
    for (top_name, targets) in transitive_groups {
        let mut eligible: Vec<(AttrPath, Vec<AttrPath>)> = targets
            .into_iter()
            .filter(|(_, paths)| paths.len() >= ax.transitive_min)
            .collect();

        if eligible.len() != 1 {
            continue;
        }

        let (target_path, paths) = eligible.pop().unwrap();
        let follow_target_str = target_path.to_flake_follows_string();

        if follow_target_str == top_name {
            continue;
        }

        let top_seg = match Segment::from_unquoted(top_name.clone()) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(
                    "Skipping toplevel follow promotion for `{top_name}`: invalid segment: {e}"
                );
                continue;
            }
        };
        // `to_flake_follows_string` produces a `/`-joined target like
        // `a/b`. Pack it into a single segment so `AttrPath::Display`
        // emits it as the quoted form `"a/b"` rather than splitting on
        // the slash as if it were a dotted path.
        let follow_target_seg = match Segment::from_unquoted(follow_target_str) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Skipping toplevel follow promotion for `{top_name}`: {e}");
                continue;
            }
        };
        let follow_target_path = AttrPath::new(follow_target_seg);
        plan.toplevel_follows
            .push((AttrPath::new(top_seg.clone()), follow_target_path));

        let top_path = AttrPath::new(top_seg);
        for path in paths {
            if plan.seen_nested.insert(path.clone()) {
                plan.to_follow.push((path, top_path.clone()));
            }
        }
    }
}

/// Turn direct-reference groups into new top-level adds and back-fill
/// `plan.to_follow` with the per-nested follows hanging off them. Promote
/// only if at least one follows can be applied (probed via a speculative
/// `apply_change` against `editor.text()`).
///
/// Runs before [`emit_transitive_promotions`] and
/// [`collect_direct_candidates`] so the names this function pushes into
/// `plan.toplevel_adds` can be folded into `ax.top_level_inputs` before
/// the later passes consult it.
fn emit_direct_promotions(
    ax: &AnalysisCtx<'_>,
    source_text: &str,
    direct_groups: HashMap<String, Vec<(AttrPath, Option<String>)>>,
    plan: &mut FollowPlan,
) {
    let mut direct_groups_sorted: Vec<_> = direct_groups.into_iter().collect();
    direct_groups_sorted.sort_by(|a, b| a.0.cmp(&b.0));
    for (canonical_name, mut entries) in direct_groups_sorted {
        if entries.len() < ax.transitive_min {
            continue;
        }

        entries.sort_by(|a, b| a.0.cmp(&b.0));

        let Some(url) = entries.iter().find_map(|(_, u)| u.clone()) else {
            continue;
        };

        let target_attr = match AttrPath::parse(&canonical_name) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    "Skipping direct-reference promotion for `{canonical_name}`: invalid attr path: {e}"
                );
                continue;
            }
        };

        let can_follow = entries.iter().any(|(path, _)| {
            let change = Change::Follows {
                input: ChangeId::new(path.clone()),
                target: target_attr.clone(),
            };
            FlakeEdit::from_text(source_text)
                .ok()
                .and_then(|mut fe| fe.apply_change(change).ok())
                .and_then(|outcome| outcome.text)
                .is_some()
        });
        if !can_follow {
            continue;
        }

        plan.toplevel_adds.push((canonical_name.clone(), url));

        // Skip entries whose ancestor is also in this group: the
        // ancestor's follow already rewrites the descendant's prefix.
        let entry_paths: HashSet<AttrPath> = entries.iter().map(|(p, _)| p.clone()).collect();
        for (path, _) in &entries {
            let mut covered_by_ancestor = false;
            let mut anc = path.parent();
            while let Some(a) = anc.clone() {
                if entry_paths.contains(&a) {
                    covered_by_ancestor = true;
                    break;
                }
                anc = a.parent();
            }
            if covered_by_ancestor {
                tracing::debug!(
                    "Skipping promotion {} -> {}: ancestor in same group covers it",
                    path,
                    canonical_name,
                );
                plan.seen_nested.insert(path.clone());
                continue;
            }
            if plan.seen_nested.insert(path.clone()) {
                plan.to_follow.push((path.clone(), target_attr.clone()));
            }
        }
    }
}

/// Drop entries from [`FollowPlan::to_follow`] whose chain is already
/// covered by the rest of the plan plus the lockfile (passed to
/// [`FollowsGraph::lock_routes_to`] as `extra_edges`).
///
/// Catches cross-entry overlaps that [`collect_direct_candidates`],
/// [`emit_direct_promotions`], and [`emit_transitive_promotions`] cannot see
/// from a single candidate's perspective.
/// Idempotent: dropping entries only shrinks `extra_edges`, so a surviving
/// entry stays surviving.
fn scrub_redundant(graph: &FollowsGraph, plan: &mut FollowPlan) {
    if plan.to_follow.is_empty() {
        return;
    }
    let mut keep: Vec<bool> = vec![true; plan.to_follow.len()];
    let mut changed = true;
    while changed {
        changed = false;
        for i in 0..plan.to_follow.len() {
            if !keep[i] {
                continue;
            }
            let (src_i, target_path) = (plan.to_follow[i].0.clone(), plan.to_follow[i].1.clone());
            let extras: Vec<(AttrPath, AttrPath)> = (0..plan.to_follow.len())
                .filter(|&j| j != i && keep[j])
                .map(|j| (plan.to_follow[j].0.clone(), plan.to_follow[j].1.clone()))
                .collect();
            if graph.lock_routes_to(&src_i, &target_path, None, &extras) {
                tracing::debug!(
                    "Scrubbing {} -> {}: redundant given the rest of the plan",
                    src_i,
                    target_path,
                );
                keep[i] = false;
                changed = true;
            }
        }
    }
    let mut keep_iter = keep.into_iter();
    plan.to_follow.retain(|_| keep_iter.next().unwrap_or(true));
}

/// Apply each scheduled change against a working text buffer, validating
/// between steps.
///
/// Lock-drift lints run once on the pre-batch text via
/// [`validate::validate_full`]. Per-step validation uses
/// [`validate::validate_speculative`], which skips them: mid-batch the
/// in-memory text contains follows the on-disk lockfile has not seen yet,
/// and re-running the lock-drift lints would flag every in-progress edit
/// as drift.
///
/// The cycle lint runs every step against the post-change `temp.curr_list()`
/// and catches any cycle introduced by the in-progress batch, skipping the
/// offending change.
fn apply_plan(
    editor: &Editor,
    state: &AppState,
    inputs: &crate::edit::InputMap,
    plan: &FollowPlan,
) -> Result<AppliedPlan> {
    let batch_lock: Option<FlakeLock> = commands::load_flake_lock(state).ok();
    apply_plan_text(&editor.text(), batch_lock.as_ref(), inputs, plan)
}

fn apply_plan_text(
    original_text: &str,
    batch_lock: Option<&FlakeLock>,
    inputs: &crate::edit::InputMap,
    plan: &FollowPlan,
) -> Result<AppliedPlan> {
    let mut current_text = original_text.to_owned();
    let mut warnings: Vec<validate::ValidationError> = Vec::new();

    // Lock-drift lints fire only on the pre-batch text. Mid-batch they would
    // flag every in-progress edit as drift against the on-disk lockfile.
    let pre_validation = validate::validate_full(&current_text, inputs, batch_lock);
    warnings.extend(pre_validation.warnings);

    // Top-level adds must precede follows that name them.
    for (id, url) in &plan.toplevel_adds {
        let change = Change::Add {
            id: Some(id.clone()),
            uri: Some(url.clone()),
            flake: true,
        };

        let mut temp = FlakeEdit::from_text(&current_text).map_err(CommandError::FlakeEdit)?;
        match temp.apply_change(change) {
            Ok(outcome) => match outcome.text {
                Some(resulting_text) => {
                    let validation = validate::validate_speculative(
                        &resulting_text,
                        temp.curr_list(),
                        batch_lock,
                    );
                    if validation.is_ok() {
                        warnings.extend(validation.warnings);
                        current_text = resulting_text;
                    } else {
                        for err in validation.errors {
                            eprintln!("Error adding top-level input {}: {}", id, err);
                        }
                    }
                }
                None => eprintln!("Could not add top-level input {}", id),
            },
            Err(e) => eprintln!("Error adding top-level input {}: {}", id, e),
        }
    }

    let mut follow_changes: Vec<(AttrPath, AttrPath)> = plan.toplevel_follows.clone();
    follow_changes.extend(plan.to_follow.iter().cloned());

    let mut applied_follows: Vec<(AttrPath, AttrPath)> = Vec::new();

    for (input_path, target) in &follow_changes {
        let change = Change::Follows {
            input: ChangeId::new(input_path.clone()),
            target: target.clone(),
        };

        let mut temp = FlakeEdit::from_text(&current_text).map_err(CommandError::FlakeEdit)?;
        match temp.apply_change(change) {
            Ok(outcome) => match outcome.text {
                Some(resulting_text) => {
                    if resulting_text == current_text {
                        continue;
                    }
                    let validation = validate::validate_speculative(
                        &resulting_text,
                        temp.curr_list(),
                        batch_lock,
                    );
                    if validation.is_ok() {
                        warnings.extend(validation.warnings);
                        current_text = resulting_text;
                        applied_follows.push((input_path.clone(), target.clone()));
                    } else {
                        for err in validation.errors {
                            eprintln!("{}", format_apply_error(input_path, &err));
                        }
                    }
                }
                None => eprintln!("Could not create follows for {}", input_path),
            },
            Err(e) => eprintln!("Error applying follows for {}: {}", input_path, e),
        }
    }

    let mut unfollowed: Vec<AttrPath> = Vec::new();

    for nested_path in &plan.to_unfollow {
        let change = Change::Remove {
            ids: vec![ChangeId::new(nested_path.clone())],
        };

        let mut temp = FlakeEdit::from_text(&current_text).map_err(CommandError::FlakeEdit)?;
        match temp.apply_change(change) {
            Ok(outcome) => {
                if let Some(resulting_text) = outcome.text {
                    let validation = validate::validate_speculative(
                        &resulting_text,
                        temp.curr_list(),
                        batch_lock,
                    );
                    if validation.is_ok() {
                        warnings.extend(validation.warnings);
                        current_text = resulting_text;
                        unfollowed.push(nested_path.clone());
                    }
                }
            }
            Err(e) => eprintln!("Error removing stale follows for {}: {}", nested_path, e),
        }
    }

    Ok(AppliedPlan {
        current_text,
        applied_follows,
        unfollowed,
        warnings,
    })
}

fn render_summary(
    editor: &Editor,
    state: &AppState,
    applied: &AppliedPlan,
    quiet: bool,
) -> Result<()> {
    if !applied.warnings.is_empty() && !quiet {
        let mut seen: HashSet<String> = HashSet::new();
        for warning in &applied.warnings {
            if seen.insert(warning_dedup_key(warning)) {
                eprintln!("warning: {}", warning);
            }
        }
    }

    if applied.applied_follows.is_empty() && applied.unfollowed.is_empty() {
        return Ok(());
    }

    if state.diff {
        let original = editor.text();
        let diff = crate::diff::Diff::new(&original, &applied.current_text);
        diff.compare();
        return Ok(());
    }

    editor.apply_or_diff(&applied.current_text, state)?;

    if quiet {
        return Ok(());
    }

    if !applied.applied_follows.is_empty() {
        println!(
            "Deduplicated {} {}.",
            applied.applied_follows.len(),
            if applied.applied_follows.len() == 1 {
                "input"
            } else {
                "inputs"
            }
        );
        for (input_path, target) in &applied.applied_follows {
            println!("  {} → {}", input_path, target);
        }
    }

    if !applied.unfollowed.is_empty() {
        println!(
            "Removed {} stale follows {}.",
            applied.unfollowed.len(),
            if applied.unfollowed.len() == 1 {
                "declaration"
            } else {
                "declarations"
            }
        );
        for path in &applied.unfollowed {
            println!("  {} (input no longer exists)", path);
        }
    }

    Ok(())
}

/// Source path of the malformed declaration named by `err`, or `None`
/// for variants that carry no source (parse errors, duplicate attributes).
fn offending_source(err: &validate::ValidationError) -> Option<&AttrPath> {
    use validate::ValidationError as V;
    match err {
        V::FollowsTargetNotToplevel { edge, .. }
        | V::FollowsStale { edge, .. }
        | V::FollowsDepthExceeded { edge, .. } => Some(&edge.source),
        V::FollowsContradiction { edges, .. } => edges.first().map(|e| &e.source),
        V::FollowsCycle { cycle, .. } => cycle.edges.first().map(|e| &e.source),
        V::FollowsStaleLock { source, .. } => Some(source),
        V::ParseError { .. } | V::DuplicateAttribute(_) => None,
    }
}

/// Format a per-step speculative validation error for stderr.
///
/// `validate_speculative` lints the whole graph, so a pre-existing
/// malformed edge surfaces in every iteration of the apply loop. When the
/// error names a source path other than `applying`, blame the malformed
/// edge's owner so the message points at the actual offender.
fn format_apply_error(applying: &AttrPath, err: &validate::ValidationError) -> String {
    match offending_source(err) {
        Some(source) if source != applying => {
            format!(
                "Malformed follows declaration in {}: {}",
                source.first(),
                err
            )
        }
        _ => format!("Error applying follows for {}: {}", applying, err),
    }
}

/// Stable identity for a follows-related warning, ignoring source-text
/// location. Two warnings with the same lint kind and key fields collapse
/// to one in the auto-follow output even if their reported lines differ
/// across iterations of [`validate::validate_full`].
fn warning_dedup_key(err: &validate::ValidationError) -> String {
    use validate::ValidationError as V;
    match err {
        V::FollowsStale { edge, .. } => format!("stale|{}|{}", edge.source, edge.follows),
        V::FollowsStaleLock {
            source,
            declared_target,
            lock_target,
            ..
        } => {
            let lock = lock_target
                .as_ref()
                .map(|t| t.to_string())
                .unwrap_or_default();
            format!("stale-lock|{source}|{declared_target}|{lock}")
        }
        other => format!("other|{other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Range;
    use crate::validate::Location;
    use clap::Parser;

    fn declared_edge(source: &str, follows: &str) -> Edge {
        Edge {
            source: AttrPath::parse(source).expect("source"),
            follows: AttrPath::parse(follows).expect("follows"),
            origin: EdgeOrigin::Declared {
                range: Range { start: 0, end: 0 },
            },
        }
    }

    fn loc() -> Location {
        Location {
            line: 60,
            column: 13,
        }
    }

    #[test]
    fn malformed_edge_blames_its_owner_not_iteration_target() {
        let applying = AttrPath::parse("browservice.flake-utils").unwrap();
        let err = validate::ValidationError::FollowsTargetNotToplevel {
            edge: declared_edge(
                "mac-app-util.cl-nix-lite",
                r#""github:verymucho/cl-nix-lite""#,
            ),
            location: loc(),
        };

        let message = format_apply_error(&applying, &err);

        assert!(
            !message.contains("browservice"),
            "must not blame iteration target, got: {message}",
        );
        assert!(
            message.contains("mac-app-util"),
            "must name the malformed edge's owner, got: {message}",
        );
    }

    #[test]
    fn self_caused_error_keeps_apply_framing() {
        let applying = AttrPath::parse("crane.nixpkgs").unwrap();
        let err = validate::ValidationError::FollowsTargetNotToplevel {
            edge: declared_edge("crane.nixpkgs", "missing"),
            location: loc(),
        };

        let message = format_apply_error(&applying, &err);

        assert!(
            message.contains("Error applying follows for crane.nixpkgs"),
            "self-caused error must keep apply framing, got: {message}",
        );
    }

    #[test]
    fn run_batch_surfaces_every_failure() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let missing_a = tmp.path().join("a/flake.nix");
        let missing_b = tmp.path().join("b/flake.nix");
        let paths = vec![missing_a.clone(), missing_b.clone()];
        let args = crate::cli::CliArgs::parse_from(["flake-edit", "follow"]);

        let err = run_batch(&paths, None, None, &args).expect_err("expected batch failure");
        let CommandError::Batch { failures } = err else {
            panic!("expected CommandError::Batch, got: {err:?}");
        };

        assert_eq!(
            failures.len(),
            2,
            "every per-file failure must reach the caller, got: {failures:?}",
        );
        let collected: Vec<&std::path::PathBuf> = failures.iter().map(|(p, _)| p).collect();
        assert!(collected.contains(&&missing_a));
        assert!(collected.contains(&&missing_b));
    }
}
