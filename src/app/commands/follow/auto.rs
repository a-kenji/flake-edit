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
use crate::lock::sizes::InputSizes;
use crate::lock::{FlakeLock, NestedInput};
use crate::validate;
use crate::walk::{Splice, flake_attr_set, rerecord_input};

use super::super::super::editor::Editor;
use super::super::super::state::AppState;
use super::super::{Error, Result};
use super::{FollowContext, load_follow_context};

mod stats;

const SENTINEL_NOTHING_TO_DEDUPLICATE: &str = "Nothing to deduplicate.";
const SENTINEL_ALREADY_DEDUPLICATED: &str = "All inputs are already deduplicated.";

fn idle_message(nested_inputs: &[NestedInput]) -> &'static str {
    if nested_inputs.is_empty() {
        SENTINEL_NOTHING_TO_DEDUPLICATE
    } else {
        SENTINEL_ALREADY_DEDUPLICATED
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportMode {
    /// Summary of the applied changes.
    Plain,
    /// Add a stats section to the summary about newly deduplicated
    /// as well as already deduplicated inputs.
    Stats,
    /// The same report as [`Self::Stats`], but without applying changes.
    DryRun,
}

impl ReportMode {
    pub fn from_flags(stats: bool, dry_run: bool) -> Self {
        if dry_run {
            Self::DryRun
        } else if stats {
            Self::Stats
        } else {
            Self::Plain
        }
    }

    fn includes_stats(self) -> bool {
        matches!(self, Self::Stats | Self::DryRun)
    }

    fn is_dry_run(self) -> bool {
        matches!(self, Self::DryRun)
    }
}

/// Entry point for `flake-edit follow` on a single in-memory flake.
pub fn run(
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
    report: ReportMode,
) -> Result<()> {
    run_impl(editor, flake_edit, state, false, report)
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
    let mut flake_edit = FlakeEdit::from_text(flake_text)?;
    let lock = FlakeLock::read_from_str(lock_text)?;
    let Some(ctx) = FollowContext::new(lock, flake_edit.list().clone()) else {
        return Ok(None);
    };
    let (lock_graph, Some(plan)) = plan_follows(&ctx, flake_text, follow_config) else {
        return Ok(None);
    };

    let applied = apply_plan_text(
        flake_text,
        &ctx.inputs,
        &ctx.nested_inputs,
        &lock_graph,
        &plan,
    )?;
    Ok((applied.current_text != flake_text).then_some(applied.current_text))
}

/// Entry point for batch mode (`flake-edit follow [PATHS...]`).
///
/// Each file is processed independently with its own [`Editor`] and
/// [`AppState`]; processing continues past per-file failures. Any
/// failures are bundled into a single [`Error::Batch`].
pub fn run_batch(
    paths: &[std::path::PathBuf],
    transitive: Option<usize>,
    depth: Option<usize>,
    report: ReportMode,
    args: &crate::cli::CliArgs,
) -> Result<()> {
    use std::path::PathBuf;

    let mut errors: Vec<(PathBuf, Box<Error>)> = Vec::new();

    for given_path in paths {
        let flake_path = if given_path.is_dir() {
            given_path.join("flake.nix")
        } else {
            given_path.clone()
        };
        let lock_path = flake_path
            .parent()
            .map(|p| p.join("flake.lock"))
            .unwrap_or_else(|| PathBuf::from("flake.lock"));

        let editor = match Editor::from_path(flake_path.clone()) {
            Ok(e) => e,
            Err(source) => {
                errors.push((
                    flake_path.clone(),
                    Box::new(Error::FlakeNotFound {
                        path: flake_path.clone(),
                        source,
                    }),
                ));
                continue;
            }
        };

        let mut flake_edit = match editor.create_flake_edit() {
            Ok(fe) => fe,
            Err(e) => {
                errors.push((flake_path.clone(), Box::new(e.into())));
                continue;
            }
        };

        let mut state = match AppState::new(flake_path.clone(), args.config().map(PathBuf::from)) {
            Ok(s) => s
                .with_diff(args.diff())
                .with_no_lock(args.no_lock())
                .with_lock_offline(true)
                .with_interactive(false)
                .with_lock_file(Some(lock_path))
                .with_no_cache(args.no_cache())
                .with_cache_path(args.cache().map(PathBuf::from)),
            Err(e) => {
                errors.push((flake_path.clone(), Box::new(e.into())));
                continue;
            }
        };

        if let Some(min) = transitive {
            state.config.follow.transitive_min = min;
        }
        if let Some(max) = depth {
            state.config.follow.max_depth = Some(max);
        }

        let quiet = !report.includes_stats();
        if !quiet && paths.len() > 1 {
            println!("# {}", flake_path.display());
        }
        if let Err(e) = run_impl(&editor, &mut flake_edit, &state, quiet, report) {
            errors.push((flake_path.clone(), Box::new(e)));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(Error::Batch { failures: errors })
    }
}

/// Inputs threaded through analysis.
///
/// `top_level_inputs` is owned because [`run_impl`] extends it with names
/// minted by [`emit_direct_promotions`] before the later passes consult it
/// (see the pass ordering in [`run_impl`]). The other fields are derived
/// once from the on-disk text and the lockfile and stay put.
struct AnalysisCtx<'a> {
    /// Lockfile entries. Replaced by [`with_promoted_subtrees`] once
    /// [`emit_direct_promotions`] has run.
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
    /// Nested inputs the user already configured in `flake.nix`.
    user_configured: &'a HashSet<AttrPath>,
    follow_config: &'a FollowConfig,
    /// Maximum depth of follows declarations to write. `None` (the default)
    /// writes follows at every depth the lockfile graph supports. `Some(n)`
    /// caps emission at depth `n`.
    max_depth: Option<usize>,
    transitive_min: usize,
}

/// Mutable plan accumulated across the collection functions.
///
/// The four output buckets (`to_follow`, `to_unfollow`, `toplevel_follows`,
/// `toplevel_adds`) feed [`apply_plan_text`]. `seen_nested` is a dedup guard
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
    toplevel_adds: Vec<Promotion>,
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

/// A new top-level input promoted from a group of nested inputs.
struct Promotion {
    name: Segment,
    url: String,
    flake: bool,
    /// The group member whose source the new input copies.
    source: AttrPath,
}

/// What [`apply_plan_text`] committed to the working text.
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
    report: ReportMode,
) -> Result<()> {
    let Some(ctx) = load_follow_context(flake_edit, state)? else {
        if !quiet {
            println!("{SENTINEL_NOTHING_TO_DEDUPLICATE}");
        }
        return Ok(());
    };

    // Stats are opt-in because sizing spawns a `nix path-info` subprocess.
    // This is potentially costly.
    let sizes = if report.includes_stats() {
        InputSizes::query(&ctx.lock, &ctx.nested_inputs)
    } else {
        InputSizes::default()
    };

    let (lock_graph, Some(plan)) = plan_follows(&ctx, &editor.text(), &state.config.follow) else {
        if !quiet {
            println!("{}", idle_message(&ctx.nested_inputs));
            render_existing_section(report, &ctx.nested_inputs, &sizes);
        }
        return Ok(());
    };

    let applied = apply_plan_text(
        &editor.text(),
        &ctx.inputs,
        &ctx.nested_inputs,
        &lock_graph,
        &plan,
    )?;
    render_summary(
        editor,
        state,
        &applied,
        quiet,
        report,
        &ctx.nested_inputs,
        &sizes,
    )?;
    if !quiet {
        render_existing_section(report, &ctx.nested_inputs, &sizes);
    }
    Ok(())
}

/// Render stats for the currently existing follow state.
fn render_existing_section(report: ReportMode, nested_inputs: &[NestedInput], sizes: &InputSizes) {
    if !report.includes_stats() {
        return;
    }
    if let Some(section) = stats::existing_summary(nested_inputs, sizes) {
        println!();
        print!("{section}");
    }
}

fn plan_follows(
    ctx: &FollowContext,
    source_text: &str,
    follow_config: &FollowConfig,
) -> (FollowsGraph, Option<FollowPlan>) {
    let lock_graph = FollowsGraph::from_lock_walk(&ctx.lock, &ctx.nested_inputs);

    let graph = FollowsGraph::from_declared_and_lock_graph(&ctx.inputs, &lock_graph);

    let plan = build_plan(
        source_text,
        &ctx.nested_inputs,
        ctx.top_level_inputs.clone(),
        &ctx.inputs,
        &graph,
        follow_config,
    );
    (lock_graph, plan)
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
    let mut user_configured: HashSet<AttrPath> = graph.declared_sources();
    user_configured.extend(inputs.values().flat_map(|input| {
        input.overrides.iter().map(|nested| {
            let mut path = AttrPath::new(input.id().clone());
            for seg in nested.segments() {
                path.push(seg.clone());
            }
            path
        })
    }));

    let transitive_min = follow_config.transitive_min();
    let max_depth = follow_config.max_depth;

    // Seeding runs against the original `graph`: the post-removal clone
    // built below would chicken-and-egg this loop.
    let to_unfollow = seed_unfollow_set(graph, max_depth);

    // Discovery must see the post-removal graph. Without this, an edge
    // marked for removal still shapes the cycle and routing checks
    // below and can suppress a candidate the apply phase would
    // otherwise unblock, forcing a second `flake-edit follow`
    // invocation to converge. The clone is a discovery-only artifact:
    // the apply phase still runs `Change::Remove` against the
    // unmodified `editor.text()`.
    let mut graph_for_discovery = graph.clone();
    graph_for_discovery.drop_edges_with_sources(&to_unfollow);

    let nested_with_promoted: Vec<NestedInput>;
    let mut ax = AnalysisCtx {
        nested_inputs,
        top_level_inputs,
        inputs,
        graph: &graph_for_discovery,
        user_configured: &user_configured,
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

        for promotion in &plan.toplevel_adds {
            ax.top_level_inputs
                .insert(promotion.name.as_str().to_string());
        }
        nested_with_promoted = with_promoted_subtrees(nested_inputs, &plan.toplevel_adds);
        ax.nested_inputs = &nested_with_promoted;

        let transitive_groups = collect_transitive_groups(&ax, &plan);
        emit_transitive_promotions(&ax, transitive_groups, &mut plan);
    }

    collect_direct_candidates(&ax, &mut plan);

    scrub_redundant(&graph_for_discovery, &mut plan);
    drop_overrides_under_redirects(&graph_for_discovery, &mut plan);

    if !plan.has_pending() {
        return None;
    }

    Some(plan)
}

/// Seed the auto-follow plan's `to_unfollow` set with sources that
/// should be removed before discovery runs.
///
/// Two removal classes are collected: stale declared edges (source
/// absent from the lockfile, including target-less `follows = ""`
/// twins that skip [`FollowsGraph::declared_edges`]) and depth-N
/// edges that the lockfile already routes via upstream propagation.
/// The [`FollowsGraph::lock_routes_to`] call passes `Some(edge)` so
/// the edge under test is excluded; without that the user's own
/// declaration counts as the route and every declared edge would
/// qualify.
///
/// Result is lex-sorted and deduplicated.
fn seed_unfollow_set(graph: &FollowsGraph, max_depth: Option<usize>) -> Vec<AttrPath> {
    let mut to_unfollow: Vec<AttrPath> = graph
        .stale_edges()
        .into_iter()
        .map(|e| e.source.clone())
        .collect();
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
        if let Some(max) = max_depth
            && edge.source.len() > max + 1
        {
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
    to_unfollow
}

/// Depth-bounded path-shape filter shared by every collection function.
///
/// Path length encodes depth: `parent.nested` is depth 1 (length 2),
/// `parent.middle.grandchild` is depth 2 (length 3). When `max_depth` is
/// `Some(n)`, the bound `len() <= n + 1` admits exactly that depth. When
/// `max_depth` is `None`, every path of length `>= 2` is admitted.
fn within_depth(path: &AttrPath, max_depth: Option<usize>) -> bool {
    if path.len() < 2 {
        return false;
    }
    max_depth.is_none_or(|m| path.len() <= m + 1)
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
        // Same-run depth-1 emissions can close a depth-N chain.
        let extra_edges: Vec<(AttrPath, AttrPath)> = if nested.path.len() >= 3 {
            plan.to_follow.to_vec()
        } else {
            Vec::new()
        };
        let Some(target_path) = resolve_direct_candidate(ax, nested, &extra_edges) else {
            continue;
        };
        plan.seen_nested.insert(nested.path.clone());
        plan.to_follow.push((nested.path.clone(), target_path));
    }
}

/// `prior_emissions` carries the same-run `to_follow` entries already
/// queued at shallower depths so [`FollowsGraph::lock_routes_to`] sees
/// them when checking depth-N candidates.
fn resolve_direct_candidate(
    ax: &AnalysisCtx<'_>,
    nested: &NestedInput,
    prior_emissions: &[(AttrPath, AttrPath)],
) -> Option<AttrPath> {
    if !within_depth(&nested.path, ax.max_depth) {
        return None;
    }
    let parent = nested.path.first().as_str();
    let nested_name = nested.path.last().as_str();
    let path_display = nested.path.to_string();

    if ax.follow_config.is_ignored(&path_display, nested_name) {
        tracing::debug!("Skipping {}: ignored by config", path_display);
        return None;
    }
    if ax.user_configured.contains(&nested.path) {
        tracing::debug!("Skipping {}: already follows in flake.nix", path_display);
        return None;
    }

    // An alias is just the fallback to the actual name.
    let target = if ax.top_level_inputs.contains(nested_name) {
        nested_name
    } else {
        ax.follow_config
            .resolve_alias(nested_name)
            .filter(|canonical| ax.top_level_inputs.contains(*canonical))?
    };

    if let Some(target_input) = ax.inputs.get(target)
        && is_follows_reference_to_parent(target_input.url(), parent)
    {
        tracing::debug!(
            "Skipping {} -> {}: would create cycle (target follows {}/...)",
            path_display,
            target,
            parent,
        );
        return None;
    }

    let target_path = match Segment::from_unquoted(target) {
        Ok(seg) => AttrPath::new(seg),
        Err(e) => {
            tracing::warn!("Skipping {path_display} -> {target}: invalid input name: {e}");
            return None;
        }
    };

    if ancestor_overrides_subtree(&nested.path, &target_path, ax.graph) {
        tracing::debug!(
            "Skipping {} -> {}: ancestor declares a different follows for the same trailing name",
            path_display,
            target,
        );
        return None;
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
        return None;
    }
    if ax
        .graph
        .lock_routes_to(&nested.path, &target_path, None, prior_emissions)
    {
        tracing::debug!(
            "Skipping {} -> {}: lockfile already routes via upstream propagation",
            path_display,
            target,
        );
        return None;
    }

    Some(target_path)
}

fn collect_transitive_groups(
    ax: &AnalysisCtx<'_>,
    plan: &FollowPlan,
) -> HashMap<String, HashMap<AttrPath, Vec<AttrPath>>> {
    let mut groups: HashMap<String, HashMap<AttrPath, Vec<AttrPath>>> = HashMap::new();

    for nested in ax.nested_inputs.iter() {
        let Some((top_level_name, transitive_target)) =
            resolve_transitive_candidate(ax, plan, nested)
        else {
            continue;
        };
        groups
            .entry(top_level_name)
            .or_default()
            .entry(transitive_target)
            .or_default()
            .push(nested.path.clone());
    }

    groups
}

fn resolve_transitive_candidate(
    ax: &AnalysisCtx<'_>,
    plan: &FollowPlan,
    nested: &NestedInput,
) -> Option<(String, AttrPath)> {
    if !within_depth(&nested.path, ax.max_depth) {
        return None;
    }
    let nested_name = nested.path.last().as_str();
    let parent = nested.path.first().as_str();
    let path_display = nested.path.to_string();

    if ax.follow_config.is_ignored(&path_display, nested_name) {
        return None;
    }
    if ax.user_configured.contains(&nested.path) || plan.seen_nested.contains(&nested.path) {
        return None;
    }
    // Handled by [`collect_direct_candidates`].
    if ax
        .top_level_inputs
        .iter()
        .any(|top| ax.follow_config.can_follow(nested_name, top))
    {
        return None;
    }

    let transitive_target = nested.follows.as_ref()?;
    if ancestor_overrides_subtree(&nested.path, transitive_target, ax.graph) {
        return None;
    }
    if transitive_target.len() < 2 {
        return None;
    }
    if transitive_target.last().as_str() == nested_name {
        return None;
    }

    let top_level_name = ax
        .follow_config
        .resolve_alias(nested_name)
        .unwrap_or(nested_name)
        .to_string();
    if ax.top_level_inputs.contains(&top_level_name) {
        return None;
    }

    if let Some(target_input) = ax.inputs.get(transitive_target.first().as_str())
        && is_follows_reference_to_parent(target_input.url(), parent)
    {
        return None;
    }

    let proposed = Edge {
        source: nested.path.clone(),
        follows: transitive_target.clone(),
        origin: EdgeOrigin::Declared {
            range: Range { start: 0, end: 0 },
        },
    };
    if ax.graph.would_create_cycle(&proposed) {
        return None;
    }

    Some((top_level_name, transitive_target.clone()))
}

/// When several parents share the same dependency (e.g. `treefmt.nixpkgs`
/// and `treefmt-nix.nixpkgs`), one can be promoted to top-level and the
/// others follow it.
fn collect_direct_groups<'a>(
    ax: &AnalysisCtx<'a>,
    plan: &FollowPlan,
) -> HashMap<String, Vec<&'a NestedInput>> {
    let mut groups: HashMap<String, Vec<&'a NestedInput>> = HashMap::new();

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
        if ax.user_configured.contains(&nested.path) || plan.seen_nested.contains(&nested.path) {
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

        groups.entry(canonical_name).or_default().push(nested);
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
    let mut transitive_groups_sorted: Vec<_> = transitive_groups.into_iter().collect();
    transitive_groups_sorted.sort_by(|a, b| a.0.cmp(&b.0));
    for (top_name, targets) in transitive_groups_sorted {
        let mut eligible: Vec<(AttrPath, Vec<AttrPath>)> = targets
            .into_iter()
            .filter(|(_, paths)| paths.len() >= ax.transitive_min)
            .collect();

        if eligible.len() != 1 {
            continue;
        }

        let (target_path, paths) = eligible.pop().unwrap();

        if target_path.to_flake_follows_string() == top_name {
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
        plan.toplevel_follows
            .push((AttrPath::new(top_seg.clone()), target_path));

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
    direct_groups: HashMap<String, Vec<&NestedInput>>,
    plan: &mut FollowPlan,
) {
    // Probes only differ in which `Change` they apply, not in the input
    // text, so parse `source_text` once and clone the syntax per probe.
    let probe_parsed = validate::ParsedSource::new(source_text);
    if !probe_parsed.parse_errors.is_empty() {
        return;
    }
    let probe_syntax = probe_parsed.syntax;

    let mut direct_groups_sorted: Vec<_> = direct_groups.into_iter().collect();
    direct_groups_sorted.sort_by(|a, b| a.0.cmp(&b.0));
    for (canonical_name, mut entries) in direct_groups_sorted {
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        let Some(promotion) = decide_direct_promotion(ax, &probe_syntax, &canonical_name, &entries)
        else {
            continue;
        };
        let target_attr = AttrPath::new(promotion.name.clone());
        plan.toplevel_adds.push(promotion);
        record_direct_promotion_entries(plan, &target_attr, &canonical_name, &entries);
    }
}

/// `probe_syntax` is the original `flake.nix` parsed by the caller
/// once and cloned per iteration. The speculative `apply_change`
/// here only checks that an edit would land; the committing pass
/// runs later against the live working text.
fn decide_direct_promotion(
    ax: &AnalysisCtx<'_>,
    probe_syntax: &rnix::SyntaxNode,
    canonical_name: &str,
    entries: &[&NestedInput],
) -> Option<Promotion> {
    if entries.len() < ax.transitive_min {
        return None;
    }
    let (source, url) = entries
        .iter()
        .filter(|n| n.path.last().as_str() == canonical_name)
        .find_map(|n| n.url.as_ref().map(|url| (*n, url)))?;
    let name = match Segment::from_unquoted(canonical_name.to_string()) {
        Ok(seg) => seg,
        Err(e) => {
            tracing::warn!(
                "Skipping direct-reference promotion for `{canonical_name}`: invalid input name: {e}"
            );
            return None;
        }
    };
    let target_attr = AttrPath::new(name.clone());
    let can_follow = entries.iter().any(|n| {
        let change = Change::Follows {
            input: ChangeId::new(n.path.clone()),
            target: target_attr.clone(),
        };
        let mut fe = FlakeEdit::from_syntax(probe_syntax.clone());
        fe.apply_change(change)
            .ok()
            .and_then(|outcome| outcome.text)
            .is_some()
    });
    if !can_follow {
        return None;
    }
    Some(Promotion {
        name,
        url: url.clone(),
        flake: source.flake,
        source: source.path.clone(),
    })
}

/// A descendant whose ancestor is also in the group is rewritten by
/// the ancestor's follow, so the descendant is dropped from
/// `plan.to_follow` but still claims `plan.seen_nested` so later
/// passes do not reclaim it.
fn record_direct_promotion_entries(
    plan: &mut FollowPlan,
    target_attr: &AttrPath,
    canonical_name: &str,
    entries: &[&NestedInput],
) {
    let entry_paths: HashSet<&AttrPath> = entries.iter().map(|n| &n.path).collect();
    for path in entries.iter().map(|n| &n.path) {
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

/// `nested_inputs` plus the lock entries each promoted input will have
/// once `nix flake lock` runs.
fn with_promoted_subtrees(
    nested_inputs: &[NestedInput],
    promotions: &[Promotion],
) -> Vec<NestedInput> {
    let mut out = nested_inputs.to_vec();
    for promotion in promotions {
        let depth = promotion.source.len();
        for nested in nested_inputs {
            if nested.follows.is_some()
                || nested.path.len() <= depth
                || !promotion.source.is_prefix_of(&nested.path)
            {
                continue;
            }
            let mut path = AttrPath::new(promotion.name.clone());
            for seg in &nested.path.segments()[depth..] {
                path.push(seg.clone());
            }
            out.push(NestedInput {
                path,
                follows: None,
                url: nested.url.clone(),
                flake: nested.flake,
            });
        }
    }
    out
}

/// Drop planned follows under an input that itself follows another input.
fn drop_overrides_under_redirects(graph: &FollowsGraph, plan: &mut FollowPlan) {
    let redirected: HashSet<AttrPath> = plan.to_follow.iter().map(|(s, _)| s.clone()).collect();
    plan.to_follow.retain(|(source, target)| {
        let mut anc = source.parent();
        while let Some(a) = anc.filter(|a| a.len() >= 2) {
            if redirected.contains(&a) || !graph.outgoing(&a).is_empty() {
                tracing::debug!("Dropping {source} -> {target}: {a} follows another input");
                return false;
            }
            anc = a.parent();
        }
        true
    });
}

/// The working tree of the apply loop and the inputs recorded from it.
struct PlanState {
    current: validate::ParsedSource,
    inputs: InputMap,
    warnings: Vec<validate::ValidationError>,
}

#[derive(Debug)]
enum StepOutcome {
    /// Validation passed and `PlanState` has been updated. `text_changed`
    /// is false when the change collapsed to the existing text; phases
    /// that record applied work key off this flag to avoid logging a
    /// follows that was already in place.
    Accepted {
        text_changed: bool,
    },
    Rejected(Vec<validate::ValidationError>),
    /// `apply_change` returned `Ok` but produced no text. Distinct from
    /// `ApplyError` because no error was raised: the underlying edit
    /// declined to mutate (e.g. removing a path that does not exist).
    NoText,
    ApplyError(crate::error::Error),
}

impl PlanState {
    fn new(current: validate::ParsedSource, warnings: Vec<validate::ValidationError>) -> Self {
        let inputs = recorded_inputs(&current.syntax);
        Self {
            current,
            inputs,
            warnings,
        }
    }

    fn try_apply_one(&mut self, change: Change) -> StepOutcome {
        let touched = match &change {
            Change::Follows { input, .. } => Some(input.input().clone()),
            _ => None,
        };
        let before = self.current.syntax.clone();
        let mut temp =
            FlakeEdit::from_syntax_and_inputs(before.clone(), std::mem::take(&mut self.inputs));
        let edited = match temp.apply_change_tree(change) {
            Ok(Some(edited)) => edited,
            Ok(None) => {
                self.restore_inputs(temp, &before);
                return StepOutcome::NoText;
            }
            Err(e) => {
                self.restore_inputs(temp, &before);
                return StepOutcome::ApplyError(e);
            }
        };
        let text_changed = edited.text() != before.text();
        let splice = Splice::find(&before, &edited);
        let parsed = validate::ParsedSource::from_edit(edited.clone(), splice.as_ref());
        let validation = validate::validate_speculative_parsed(&parsed, temp.curr_list());
        if !validation.is_ok() {
            self.restore_inputs(temp, &before);
            return StepOutcome::Rejected(validation.errors);
        }

        let inputs = match (temp.into_inputs_of(&before), touched, splice) {
            (Some(mut inputs), Some(id), Some(splice)) if parsed.syntax == edited => {
                match flake_attr_set(&edited) {
                    Some(attr_set) => {
                        let (start, delta) = splice.replaced_bytes();
                        rerecord_input(&mut inputs, &attr_set, &id, start, delta);
                        inputs
                    }
                    None => recorded_inputs(&edited),
                }
            }
            _ => recorded_inputs(&parsed.syntax),
        };
        debug_assert_eq!(inputs, recorded_inputs(&parsed.syntax));
        self.inputs = inputs;
        self.warnings.extend(validation.warnings);
        self.current = parsed;
        StepOutcome::Accepted { text_changed }
    }

    fn restore_inputs(&mut self, temp: FlakeEdit, before: &rnix::SyntaxNode) {
        self.inputs = temp
            .into_inputs_of(before)
            .unwrap_or_else(|| recorded_inputs(before));
    }

    fn text(&self) -> String {
        self.current.syntax.to_string()
    }
}

fn recorded_inputs(syntax: &rnix::SyntaxNode) -> InputMap {
    let mut flake_edit = FlakeEdit::from_syntax(syntax.clone());
    flake_edit.list();
    flake_edit.into_inputs_of(syntax).unwrap_or_default()
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
fn apply_plan_text(
    original_text: &str,
    inputs: &crate::edit::InputMap,
    nested_inputs: &[NestedInput],
    lock_graph: &FollowsGraph,
    plan: &FollowPlan,
) -> Result<AppliedPlan> {
    let current_parsed = validate::ParsedSource::new(original_text);
    if !current_parsed.parse_errors.is_empty() {
        return Err(Error::Flake(crate::error::Error::Validation(
            current_parsed.parse_errors.clone(),
        )));
    }

    let mut warnings: Vec<validate::ValidationError> = Vec::new();

    // Lock-drift lints fire only on the pre-batch text. Mid-batch they would
    // flag every in-progress edit as drift against the on-disk lockfile. The
    // prebuilt `lock_graph` and `nested_inputs` avoid re-walking `flake.lock`.
    let pre_validation = validate::validate_full_with_lock_graph(
        &current_parsed,
        inputs,
        Some(lock_graph),
        nested_inputs,
    );
    warnings.extend(pre_validation.warnings);

    let mut state = PlanState::new(current_parsed, warnings);

    // Top-level adds must precede follows that name them.
    apply_toplevel_adds(plan, &mut state);
    let applied_follows = apply_follow_changes(plan, &mut state);
    let unfollowed = apply_unfollow_changes(plan, &mut state);

    Ok(AppliedPlan {
        current_text: state.text(),
        applied_follows,
        unfollowed,
        warnings: state.warnings,
    })
}

fn apply_toplevel_adds(plan: &FollowPlan, state: &mut PlanState) {
    for Promotion {
        name: id,
        url,
        flake,
        ..
    } in &plan.toplevel_adds
    {
        let change = Change::Add {
            id: Some(ChangeId::new(AttrPath::new(id.clone()))),
            uri: Some(url.clone()),
            flake: *flake,
        };
        match state.try_apply_one(change) {
            StepOutcome::Accepted { .. } => {}
            StepOutcome::Rejected(errors) => {
                for err in errors {
                    tracing::error!("could not add top-level input {id}: {err}");
                }
            }
            StepOutcome::NoText => {
                tracing::error!("could not add top-level input {id}");
            }
            StepOutcome::ApplyError(e) => {
                tracing::error!("could not add top-level input {id}: {e}");
            }
        }
    }
}

fn apply_follow_changes(plan: &FollowPlan, state: &mut PlanState) -> Vec<(AttrPath, AttrPath)> {
    let mut follow_changes: Vec<(AttrPath, AttrPath)> = plan.toplevel_follows.clone();
    follow_changes.extend(plan.to_follow.iter().cloned());

    let mut applied_follows: Vec<(AttrPath, AttrPath)> = Vec::new();
    for (input_path, target) in &follow_changes {
        let change = Change::Follows {
            input: ChangeId::new(input_path.clone()),
            target: target.clone(),
        };
        match state.try_apply_one(change) {
            StepOutcome::Accepted { text_changed: true } => {
                applied_follows.push((input_path.clone(), target.clone()));
            }
            // A Change::Follows that produced identical text means the
            // declaration was already in place; recording it as a fresh
            // application would inflate the success summary with a no-op.
            StepOutcome::Accepted {
                text_changed: false,
            } => {}
            StepOutcome::Rejected(errors) => {
                for err in errors {
                    tracing::error!("{}", format_apply_error(input_path, &err));
                }
            }
            StepOutcome::NoText => {
                tracing::error!("could not create follows for {input_path}");
            }
            StepOutcome::ApplyError(e) => {
                tracing::error!("could not apply follows for {input_path}: {e}");
            }
        }
    }
    applied_follows
}

fn apply_unfollow_changes(plan: &FollowPlan, state: &mut PlanState) -> Vec<AttrPath> {
    let mut unfollowed: Vec<AttrPath> = Vec::new();
    for nested_path in &plan.to_unfollow {
        let change = Change::Remove {
            ids: vec![ChangeId::new(nested_path.clone())],
        };
        match state.try_apply_one(change) {
            StepOutcome::Accepted { .. } => unfollowed.push(nested_path.clone()),
            // Validation failures and missing text are silently dropped:
            // unfollow is a best-effort cleanup pass and a stale source
            // that fails to remove is no worse than leaving it in place.
            StepOutcome::Rejected(_) | StepOutcome::NoText => {}
            StepOutcome::ApplyError(e) => {
                tracing::error!("could not remove stale follows for {nested_path}: {e}");
            }
        }
    }
    unfollowed
}

fn render_summary(
    editor: &Editor,
    state: &AppState,
    applied: &AppliedPlan,
    quiet: bool,
    report: ReportMode,
    nested_inputs: &[NestedInput],
    sizes: &InputSizes,
) -> Result<()> {
    if !applied.warnings.is_empty() && !quiet {
        let mut seen: HashSet<String> = HashSet::new();
        for warning in &applied.warnings {
            if seen.insert(warning_dedup_key(warning)) {
                eprintln!("warning: {}", warning);
            }
        }
    }

    // Empty plans short-circuit earlier (see [`build_plan`]).
    if applied.current_text == editor.text() {
        if !quiet {
            println!("{}", idle_message(nested_inputs));
        }
        return Ok(());
    }

    let stats_on = report.includes_stats();
    let wrote = !state.diff && !report.is_dry_run();
    if wrote {
        editor.apply_or_diff(&applied.current_text, state)?;
    } else {
        let original = editor.text();
        crate::diff::Diff::new(&original, &applied.current_text).compare();
        if !stats_on {
            return Ok(());
        }
    }

    if quiet {
        return Ok(());
    }

    print!(
        "{}",
        stats::applied_summary(applied, wrote, stats_on, nested_inputs, sizes)
    );

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
        V::FollowsStaleLock { source_path, .. } => Some(source_path),
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
        V::FollowsStale { edge, .. } => format!(
            "stale|{}|{}",
            edge.source,
            edge.follows.to_flake_follows_string()
        ),
        V::FollowsStaleLock {
            source_path,
            declared_target,
            lock_target,
            ..
        } => {
            let lock = lock_target
                .as_ref()
                .map_or_default(|t| t.to_flake_follows_string());
            format!(
                "stale-lock|{source_path}|{}|{lock}",
                declared_target.to_flake_follows_string()
            )
        }
        other => format!("other|{other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::FollowConfig;
    use crate::input::{Follows, Input, Range};
    use crate::validate::Location;
    use clap::Parser;
    use std::assert_matches;

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

    fn seg(s: &str) -> Segment {
        Segment::from_unquoted(s).unwrap()
    }

    fn ap(s: &str) -> AttrPath {
        AttrPath::parse(s).unwrap()
    }

    fn input_with_follows(id: &str, follows: Vec<(AttrPath, Option<AttrPath>)>) -> Input {
        let mut input = Input::new(seg(id));
        for (path, target) in follows {
            input.follows.push(Follows::Indirect {
                path,
                target,
                range: Range::default(),
            });
        }
        input.range = Range { start: 1, end: 2 };
        input
    }

    fn make_input_map(items: Vec<Input>) -> InputMap {
        let mut map = InputMap::new();
        for item in items {
            map.insert(item.id().as_str().to_string(), item);
        }
        map
    }

    /// Lockfile that places `parent.middle` and `parent.middle.nixpkgs`
    /// in `resolved_universe` so neither is flagged stale by
    /// [`seed_unfollow_set`]. `home-manager` and `nixos-cosmic` are locked
    /// without nested inputs, so their declared follows are stale.
    fn parent_middle_lock() -> FlakeLock {
        let lock_text = r#"{
  "nodes": {
    "top": {
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "a", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "nixpkgs_2": {
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "b", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "middle": {
      "inputs": { "nixpkgs": "nixpkgs_2" },
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "c", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "parent": {
      "inputs": { "middle": "middle" },
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "d", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "home-manager": {
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "e", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "nixos-cosmic": {
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "f", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "root": {
      "inputs": { "top": "top", "parent": "parent", "home-manager": "home-manager", "nixos-cosmic": "nixos-cosmic" }
    }
  },
  "root": "root",
  "version": 7
}"#;
        FlakeLock::read_from_str(lock_text).unwrap()
    }

    #[test]
    fn seed_unfollow_set_empty_graph_returns_empty() {
        let graph = FollowsGraph::default();
        let result = seed_unfollow_set(&graph, Some(2));
        assert_eq!(result, Vec::<AttrPath>::new());
    }

    #[test]
    fn seed_unfollow_set_collects_stale_declared_edge() {
        // `home-manager.nixpkgs` is declared with a target, but the
        // locked `home-manager` has no such input, so
        // [`FollowsGraph::stale_edges`] flags the source.
        let inputs = make_input_map(vec![input_with_follows(
            "home-manager",
            vec![(ap("nixpkgs"), Some(ap("nixpkgs")))],
        )]);
        let graph = FollowsGraph::from_flake(&inputs, &parent_middle_lock());
        let result = seed_unfollow_set(&graph, Some(2));
        assert_eq!(result, vec![ap("home-manager.nixpkgs")]);
    }

    #[test]
    fn seed_unfollow_set_collects_stale_nulled_source() {
        // `follows = ""` records the source on
        // [`FollowsGraph::declared_nulled`], not on
        // [`FollowsGraph::declared_edges`]. Without a lockfile entry the
        // helper must still surface it via
        // [`FollowsGraph::stale_nulled_sources`].
        let inputs = make_input_map(vec![input_with_follows(
            "home-manager",
            vec![(ap("nixpkgs"), None)],
        )]);
        let graph = FollowsGraph::from_flake(&inputs, &parent_middle_lock());
        let result = seed_unfollow_set(&graph, Some(2));
        assert_eq!(result, vec![ap("home-manager.nixpkgs")]);
    }

    #[test]
    fn seed_unfollow_set_marks_redundant_depth_n_edge() {
        // Ancestor `parent.middle -> top` rewrites
        // `parent.middle.nixpkgs` to `top.nixpkgs`, so the depth-2
        // declaration `parent.middle.nixpkgs -> top.nixpkgs` is
        // already covered by upstream propagation.
        let inputs = make_input_map(vec![input_with_follows(
            "parent",
            vec![
                (ap("middle"), Some(ap("top"))),
                (ap("middle.nixpkgs"), Some(ap("top.nixpkgs"))),
            ],
        )]);
        let graph = FollowsGraph::from_flake(&inputs, &parent_middle_lock());
        let result = seed_unfollow_set(&graph, Some(2));
        assert_eq!(result, vec![ap("parent.middle.nixpkgs")]);
    }

    #[test]
    fn seed_unfollow_set_mixed_sources_are_sorted_and_deduped() {
        let inputs = make_input_map(vec![
            input_with_follows("home-manager", vec![(ap("nixpkgs"), None)]),
            input_with_follows("nixos-cosmic", vec![(ap("nixpkgs"), Some(ap("nixpkgs")))]),
            input_with_follows(
                "parent",
                vec![
                    (ap("middle"), Some(ap("top"))),
                    (ap("middle.nixpkgs"), Some(ap("top.nixpkgs"))),
                ],
            ),
        ]);
        let graph = FollowsGraph::from_flake(&inputs, &parent_middle_lock());
        let result = seed_unfollow_set(&graph, Some(2));
        assert_eq!(
            result,
            vec![
                ap("home-manager.nixpkgs"),
                ap("nixos-cosmic.nixpkgs"),
                ap("parent.middle.nixpkgs"),
            ],
        );
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

        let err = run_batch(&paths, None, None, ReportMode::Plain, &args)
            .expect_err("expected batch failure");
        let Error::Batch { failures } = err else {
            panic!("expected Error::Batch, got: {err:?}");
        };

        assert_eq!(
            failures.len(),
            2,
            "every per-file failure must reach the caller, got: {failures:?}",
        );
        for (path, err) in &failures {
            assert_matches!(
                err.as_ref(),
                Error::FlakeNotFound { .. },
                "expected FlakeNotFound for missing flake.nix at {}, got {err:?}",
                path.display(),
            );
        }
        let collected: Vec<&std::path::PathBuf> = failures.iter().map(|(p, _)| p).collect();
        assert!(collected.contains(&&missing_a));
        assert!(collected.contains(&&missing_b));
    }

    /// A single `follow` invocation walks the immutable `flake.lock` exactly
    /// once. `crane.nixpkgs` is a separate `nixpkgs` copy, so the planner
    /// emits `crane.nixpkgs -> nixpkgs` and the full apply path runs,
    /// exercising discovery, lock-drift validation, and the per-step
    /// speculative checks from one shared lock walk.
    #[test]
    fn follow_walks_lockfile_once_per_invocation() {
        let flake = r#"{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
    crane.url = "github:ipetkov/crane";
  };
  outputs = { ... }: { };
}
"#;
        let lock = r#"{
  "nodes": {
    "nixpkgs": {
      "locked": { "lastModified": 1, "narHash": "", "owner": "nixos", "repo": "nixpkgs", "rev": "aaa", "type": "github" },
      "original": { "owner": "nixos", "repo": "nixpkgs", "type": "github" }
    },
    "nixpkgs_2": {
      "locked": { "lastModified": 1, "narHash": "", "owner": "nixos", "repo": "nixpkgs", "rev": "bbb", "type": "github" },
      "original": { "owner": "nixos", "repo": "nixpkgs", "type": "github" }
    },
    "crane": {
      "inputs": { "nixpkgs": "nixpkgs_2" },
      "locked": { "lastModified": 1, "narHash": "", "owner": "ipetkov", "repo": "crane", "rev": "ccc", "type": "github" },
      "original": { "owner": "ipetkov", "repo": "crane", "type": "github" }
    },
    "root": {
      "inputs": { "nixpkgs": "nixpkgs", "crane": "crane" }
    }
  },
  "root": "root",
  "version": 7
}"#;

        crate::lock::NESTED_INPUTS_CALLS.with(|c| c.set(0));
        let out = run_in_memory(flake, lock, &FollowConfig::default()).expect("run_in_memory");
        let walks = crate::lock::NESTED_INPUTS_CALLS.with(|c| c.get());

        assert!(
            out.is_some(),
            "fixture must produce a dedup so the apply path runs; got no change",
        );
        assert_eq!(
            walks, 1,
            "nested_inputs must be walked exactly once per follow invocation, got {walks}",
        );
    }

    #[test]
    fn run_in_memory_prunes_stale_follows_without_nested_inputs() {
        let flake = include_str!("../../../../tests/fixtures/stale_lockfile_only.flake.nix");
        let lock = include_str!("../../../../tests/fixtures/stale_lockfile_only.flake.lock");

        let out = run_in_memory(flake, lock, &FollowConfig::default()).expect("run_in_memory");

        let expected = flake.replace(
            "    foo.inputs.bar.follows = \"nixpkgs\";\n    foo.inputs.baz.follows = \"nixpkgs\";\n",
            "",
        );
        assert_eq!(out, Some(expected));
    }

    /// Lock where `atuin` locks its own copy of `flake-parts` next to the
    /// top-level one, the shape the dedup planner folds into a follows.
    const NESTED_COPY_LOCK: &str = r#"{
  "nodes": {
    "atuin": {
      "inputs": { "flake-parts": "flake-parts_2" },
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "atuin", "rev": "aaa", "type": "github" },
      "original": { "owner": "o", "repo": "atuin", "type": "github" }
    },
    "flake-parts": {
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "flake-parts", "rev": "fff", "type": "github" },
      "original": { "owner": "o", "repo": "flake-parts", "type": "github" }
    },
    "flake-parts_2": {
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "flake-parts", "rev": "fff", "type": "github" },
      "original": { "owner": "o", "repo": "flake-parts", "type": "github" }
    },
    "root": {
      "inputs": { "atuin": "atuin", "flake-parts": "flake-parts" }
    }
  },
  "root": "root",
  "version": 7
}"#;

    fn flake_with_atuin_lines(atuin_lines: &str) -> String {
        format!(
            r#"{{
  inputs = {{
    flake-parts.url = "github:o/flake-parts";
    atuin = {{
      url = "github:o/atuin";
{atuin_lines}    }};
  }};
  outputs = _: {{ }};
}}"#
        )
    }

    fn follow_with_nested_copy_lock(flake: &str) -> Option<String> {
        run_in_memory(flake, NESTED_COPY_LOCK, &FollowConfig::default()).expect("run_in_memory")
    }

    #[test]
    fn follow_dedups_nested_input_without_override() {
        assert_eq!(
            follow_with_nested_copy_lock(&flake_with_atuin_lines("")),
            Some(flake_with_atuin_lines(
                "      inputs.flake-parts.follows = \"flake-parts\";\n"
            ))
        );
    }

    #[test]
    fn follow_skips_nested_input_overridden_with_flake_false() {
        let flake = flake_with_atuin_lines("      inputs.flake-parts.flake = false;\n");
        assert_eq!(follow_with_nested_copy_lock(&flake), None);
    }

    #[test]
    fn follow_skips_nested_input_with_url_override() {
        let flake =
            flake_with_atuin_lines("      inputs.flake-parts.url = \"github:o/flake-parts/v1\";\n");
        assert_eq!(follow_with_nested_copy_lock(&flake), None);
    }

    /// Lock for the existing-follows tests: `a` has nested `t` (its own
    /// node `t_2`) and `n`, `n` has `t` following nothing, and a top-level
    /// `t` exists.
    const EXISTING_FOLLOWS_LOCK: &str = r#"{
  "nodes": {
    "a": {
      "inputs": { "n": "n", "t": "t_2" },
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "a", "rev": "aaa", "type": "github" },
      "original": { "owner": "o", "repo": "a", "type": "github" }
    },
    "n": {
      "inputs": { "t": [] },
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "n", "rev": "nnn", "type": "github" },
      "original": { "owner": "o", "repo": "n", "type": "github" }
    },
    "t": {
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "t", "rev": "ttt", "type": "github" },
      "original": { "owner": "o", "repo": "t", "type": "github" }
    },
    "t_2": {
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "t", "rev": "ttt", "type": "github" },
      "original": { "owner": "o", "repo": "t", "type": "github" }
    },
    "root": {
      "inputs": { "a": "a", "t": "t" }
    }
  },
  "root": "root",
  "version": 7
}"#;

    fn follow_with_existing_follows_lock(flake: &str) -> Option<String> {
        run_in_memory(flake, EXISTING_FOLLOWS_LOCK, &FollowConfig::default())
            .expect("run_in_memory")
    }

    #[test]
    fn follow_respects_existing_follows_in_inputs_block_before_url() {
        let flake = r#"{
  inputs = {
    a = {
      inputs = { t = { follows = "x"; }; };
      url = "github:o/a";
    };
    t.url = "github:o/t";
  };
  outputs = _: { };
}"#;
        assert_eq!(follow_with_existing_follows_lock(flake), None);
    }

    #[test]
    fn follow_respects_existing_follows_in_nested_inputs_attrset() {
        let flake = r#"{
  inputs = {
    a = {
      url = "github:o/a";
      inputs = {
        n.inputs = { t.follows = ""; };
      };
    };
    t.url = "github:o/t";
  };
  outputs = _: { };
}"#;
        let expected = flake.replace(
            "        n.inputs = { t.follows = \"\"; };\n",
            "        n.inputs = { t.follows = \"\"; };\n        t.follows = \"t\";\n",
        );
        assert_eq!(
            follow_with_existing_follows_lock(flake).as_deref(),
            Some(expected.as_str())
        );
    }

    fn nested_input(path: &str, follows: Option<&str>, url: Option<&str>) -> NestedInput {
        NestedInput {
            path: ap(path),
            follows: follows.map(ap),
            url: url.map(ToOwned::to_owned),
            flake: true,
        }
    }

    struct CtxFixture<'a> {
        inputs: InputMap,
        graph: FollowsGraph,
        user_configured: HashSet<AttrPath>,
        follow_config: FollowConfig,
        nested_inputs: &'a [NestedInput],
        top_level_inputs: HashSet<String>,
        max_depth: Option<usize>,
        transitive_min: usize,
    }

    impl<'a> CtxFixture<'a> {
        fn new(nested_inputs: &'a [NestedInput], top_level: &[&str]) -> Self {
            Self {
                inputs: InputMap::new(),
                graph: FollowsGraph::default(),
                user_configured: HashSet::new(),
                follow_config: FollowConfig::default(),
                nested_inputs,
                top_level_inputs: top_level.iter().map(|s| (*s).to_string()).collect(),
                max_depth: Some(1),
                transitive_min: 2,
            }
        }

        fn ctx(&self) -> AnalysisCtx<'_> {
            AnalysisCtx {
                nested_inputs: self.nested_inputs,
                top_level_inputs: self.top_level_inputs.clone(),
                inputs: &self.inputs,
                graph: &self.graph,
                user_configured: &self.user_configured,
                follow_config: &self.follow_config,
                max_depth: self.max_depth,
                transitive_min: self.transitive_min,
            }
        }
    }

    #[test]
    fn resolve_direct_candidate_returns_target_when_eligible() {
        let nested = vec![nested_input("home-manager.nixpkgs", None, None)];
        let mut fx = CtxFixture::new(&nested, &["nixpkgs", "home-manager"]);
        fx.inputs = make_input_map(vec![Input::new(seg("nixpkgs"))]);

        let result = resolve_direct_candidate(&fx.ctx(), &nested[0], &[]);

        assert_eq!(result, Some(ap("nixpkgs")));
    }

    /// With the default alias `nixpkgs = ["nixpkgs-lib"]`, both top-level
    /// `nixpkgs-lib` (exact) and `nixpkgs` (alias) accept a nested
    /// `nixpkgs-lib`.
    #[test]
    fn resolve_direct_candidate_prefers_exact_match_over_alias() {
        let nested = vec![nested_input("foo.nixpkgs-lib", None, None)];
        for _ in 0..64 {
            let fx = CtxFixture::new(&nested, &["nixpkgs", "nixpkgs-lib", "foo"]);

            let result = resolve_direct_candidate(&fx.ctx(), &nested[0], &[]);

            assert_eq!(result, Some(ap("nixpkgs-lib")));
        }
    }

    #[test]
    fn resolve_direct_candidate_skips_when_no_top_level_match() {
        let nested = vec![nested_input("home-manager.nixpkgs", None, None)];
        let fx = CtxFixture::new(&nested, &["home-manager"]);

        let result = resolve_direct_candidate(&fx.ctx(), &nested[0], &[]);

        assert_eq!(result, None);
    }

    #[test]
    fn resolve_transitive_candidate_returns_target_when_eligible() {
        // The self-follow guard compares the target's trailing
        // segment to the nested input's leaf name, so the fixture
        // chooses `top.bar` against `parent.foo` to keep them
        // distinct.
        let nested = vec![nested_input("parent.foo", Some("top.bar"), None)];
        let fx = CtxFixture::new(&nested, &["parent", "top"]);
        let plan = FollowPlan::default();

        let result = resolve_transitive_candidate(&fx.ctx(), &plan, &nested[0]);

        assert_eq!(result, Some(("foo".to_string(), ap("top.bar"))));
    }

    #[test]
    fn resolve_transitive_candidate_skips_self_follow() {
        // Promoting `parent.nixpkgs -> other.nixpkgs` into a group
        // would amount to a no-op rename of the trailing segment, so
        // the helper rejects it even though the parents differ.
        let nested = vec![nested_input("parent.nixpkgs", Some("other.nixpkgs"), None)];
        let fx = CtxFixture::new(&nested, &["parent", "other"]);
        let plan = FollowPlan::default();

        let result = resolve_transitive_candidate(&fx.ctx(), &plan, &nested[0]);

        assert_eq!(result, None);
    }

    #[test]
    fn resolve_transitive_candidate_skips_when_already_seen() {
        // The grouping pass shares `seen_nested` with the direct
        // promotion pass; a path another phase already claimed must
        // not produce a second group entry here.
        let nested = vec![nested_input(
            "parent.flake-utils",
            Some("top.flake-utils"),
            None,
        )];
        let fx = CtxFixture::new(&nested, &["parent", "top"]);
        let plan = FollowPlan {
            seen_nested: std::iter::once(ap("parent.flake-utils")).collect(),
            ..FollowPlan::default()
        };

        let result = resolve_transitive_candidate(&fx.ctx(), &plan, &nested[0]);

        assert_eq!(result, None);
    }

    #[test]
    fn record_direct_promotion_entries_pushes_orphan_paths() {
        let nested = [
            nested_input("crane.flake-utils", None, None),
            nested_input("treefmt.flake-utils", None, None),
        ];
        let entries: Vec<&NestedInput> = nested.iter().collect();
        let target = ap("flake-utils");
        let mut plan = FollowPlan::default();

        record_direct_promotion_entries(&mut plan, &target, "flake-utils", &entries);

        assert_eq!(
            plan.to_follow,
            vec![
                (ap("crane.flake-utils"), ap("flake-utils")),
                (ap("treefmt.flake-utils"), ap("flake-utils")),
            ],
        );
        assert!(plan.seen_nested.contains(&ap("crane.flake-utils")));
        assert!(plan.seen_nested.contains(&ap("treefmt.flake-utils")));
    }

    #[test]
    fn record_direct_promotion_entries_skips_descendants_covered_by_ancestor() {
        let nested = [
            nested_input("crane.flake-utils", None, None),
            nested_input("crane.flake-utils.nested", None, None),
        ];
        let entries: Vec<&NestedInput> = nested.iter().collect();
        let target = ap("flake-utils");
        let mut plan = FollowPlan::default();

        record_direct_promotion_entries(&mut plan, &target, "flake-utils", &entries);

        assert_eq!(
            plan.to_follow,
            vec![(ap("crane.flake-utils"), ap("flake-utils"))],
        );
        assert!(
            plan.seen_nested.contains(&ap("crane.flake-utils.nested")),
            "ancestor-covered descendant must still claim seen_nested",
        );
    }

    #[test]
    fn resolve_direct_candidate_skips_existing_follow() {
        let nested = vec![nested_input("home-manager.nixpkgs", None, None)];
        let mut fx = CtxFixture::new(&nested, &["nixpkgs", "home-manager"]);
        fx.inputs = make_input_map(vec![Input::new(seg("nixpkgs"))]);
        fx.user_configured = std::iter::once(ap("home-manager.nixpkgs")).collect();

        let result = resolve_direct_candidate(&fx.ctx(), &nested[0], &[]);

        assert_eq!(result, None);
    }

    fn fresh_state(text: &str) -> PlanState {
        let parsed = validate::ParsedSource::new(text);
        assert!(
            parsed.parse_errors.is_empty(),
            "test fixture must parse cleanly, got: {:?}",
            parsed.parse_errors,
        );
        PlanState::new(parsed, Vec::new())
    }

    #[test]
    fn apply_toplevel_adds_inserts_new_input() {
        let original = r#"{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs";
  };
  outputs = _: { };
}
"#;
        let plan = FollowPlan {
            toplevel_adds: vec![Promotion {
                name: seg("flake-utils"),
                url: "github:numtide/flake-utils".to_string(),
                flake: true,
                source: ap("crane.flake-utils"),
            }],
            ..FollowPlan::default()
        };
        let mut state = fresh_state(original);

        apply_toplevel_adds(&plan, &mut state);

        assert!(
            state
                .text()
                .contains(r#"flake-utils.url = "github:numtide/flake-utils""#),
            "added input declaration must appear verbatim, got:\n{}",
            state.text(),
        );
    }

    #[test]
    fn apply_follow_changes_records_accepted() {
        let original = r#"{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs";
    home-manager.url = "github:nix-community/home-manager";
  };
  outputs = _: { };
}
"#;
        let plan = FollowPlan {
            to_follow: vec![(ap("home-manager.nixpkgs"), ap("nixpkgs"))],
            ..FollowPlan::default()
        };
        let mut state = fresh_state(original);

        let applied = apply_follow_changes(&plan, &mut state);

        assert_eq!(
            applied,
            vec![(ap("home-manager.nixpkgs"), ap("nixpkgs"))],
            "happy-path follow must be recorded as applied",
        );
        assert!(
            state
                .text()
                .contains("home-manager.inputs.nixpkgs.follows = \"nixpkgs\""),
            "follows declaration must be written, got:\n{}",
            state.text(),
        );
    }

    #[test]
    fn apply_follow_changes_skips_no_op_text() {
        // Source already contains the exact declaration the plan asks
        // for. The phase must distinguish "applied a fresh follows"
        // from "the follows was already in place" so the success
        // summary does not double-count.
        let original = r#"{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs";
    home-manager.url = "github:nix-community/home-manager";
    home-manager.inputs.nixpkgs.follows = "nixpkgs";
  };
  outputs = _: { };
}
"#;
        let plan = FollowPlan {
            to_follow: vec![(ap("home-manager.nixpkgs"), ap("nixpkgs"))],
            ..FollowPlan::default()
        };
        let mut state = fresh_state(original);

        let applied = apply_follow_changes(&plan, &mut state);

        assert!(
            applied.is_empty(),
            "no-op follow must not be recorded, got: {applied:?}",
        );
        assert_eq!(
            state.text(),
            original,
            "text must be byte-equal to original on no-op",
        );
    }

    #[test]
    fn apply_unfollow_changes_removes_stale() {
        let original = r#"{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs";
    home-manager.url = "github:nix-community/home-manager";
    home-manager.inputs.nixpkgs.follows = "nixpkgs";
  };
  outputs = _: { };
}
"#;
        let plan = FollowPlan {
            to_unfollow: vec![ap("home-manager.nixpkgs")],
            ..FollowPlan::default()
        };
        let mut state = fresh_state(original);

        let unfollowed = apply_unfollow_changes(&plan, &mut state);

        assert_eq!(
            unfollowed,
            vec![ap("home-manager.nixpkgs")],
            "removed path must be reported",
        );
        assert!(
            !state.text().contains("home-manager.inputs.nixpkgs.follows"),
            "stale follows line must be gone, got:\n{}",
            state.text(),
        );
    }

    #[test]
    fn apply_unfollow_changes_skips_missing_path() {
        // `to_unfollow` is seeded from a graph view that may disagree
        // with the on-disk text after earlier phases mutate it. The
        // phase must drop entries whose source no longer resolves, so
        // the success summary cannot grow phantom "removed" lines.
        let original = r#"{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs";
  };
  outputs = _: { };
}
"#;
        let plan = FollowPlan {
            to_unfollow: vec![ap("does-not-exist")],
            ..FollowPlan::default()
        };
        let mut state = fresh_state(original);

        let unfollowed = apply_unfollow_changes(&plan, &mut state);

        assert!(
            unfollowed.is_empty(),
            "missing path must not be reported as removed, got: {unfollowed:?}",
        );
        assert_eq!(
            state.text(),
            original,
            "text must be byte-equal when no path was removed",
        );
    }

    #[test]
    fn try_apply_one_leaves_state_on_no_text() {
        let original = r#"{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs";
  };
  outputs = _: { };
}
"#;
        let mut state = fresh_state(original);
        let change = Change::Remove {
            ids: vec![ChangeId::new(ap("does-not-exist"))],
        };

        let outcome = state.try_apply_one(change);

        assert_matches!(
            outcome,
            StepOutcome::NoText,
            "expected NoText for a remove against an absent source",
        );
        assert_eq!(
            state.text(),
            original,
            "state must be untouched on a non-Accepted outcome",
        );
    }

    #[test]
    fn alias_only_group_is_not_promoted() {
        // Both members are `nixpkgs-lib`, which the default aliases fold
        // into a `nixpkgs` group. Neither is nixpkgs, so there is no
        // source to promote a top-level nixpkgs from.
        let flake = r#"{
  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    nix-colors.url = "github:misterio77/nix-colors";
  };
  outputs = _: { };
}
"#;
        let lock = r#"{
  "nodes": {
    "flake-parts_3": {
      "inputs": { "nixpkgs-lib": "nixpkgs-lib_2" },
      "locked": { "lastModified": 1790898470, "narHash": "sha256-zTwuwezD0w3hpga6a6P8emidzAZFyWqlNeRBlpWOOl8=", "owner": "hercules-ci", "repo": "flake-parts", "rev": "024633cd702b10285db5cb19b40ad48d2399ba60", "type": "github" },
      "original": { "owner": "hercules-ci", "repo": "flake-parts", "type": "github" }
    },
    "nix-colors": {
      "inputs": { "nixpkgs-lib": "nixpkgs-lib_3" },
      "locked": { "lastModified": 1707825078, "narHash": "sha256-hTfge2J2W+42SZ7VHXkf4kjU+qzFqPeC9k66jAUBMHk=", "owner": "misterio77", "repo": "nix-colors", "rev": "b01f024090d2c4fc3152cd0cf12027a7b8453ba1", "type": "github" },
      "original": { "owner": "misterio77", "repo": "nix-colors", "type": "github" }
    },
    "nixpkgs-lib_2": {
      "locked": { "lastModified": 1790477367, "narHash": "sha256-oujbxmVKDhci2E4EM3KyAbRUgp5/IsCoiA/LfEF7OOk=", "owner": "nix-community", "repo": "nixpkgs.lib", "rev": "f7cd230690d9fc982d06129f9189b101fddcfa3b", "type": "github" },
      "original": { "owner": "nix-community", "repo": "nixpkgs.lib", "type": "github" }
    },
    "nixpkgs-lib_3": {
      "locked": { "lastModified": 1697935651, "narHash": "sha256-qOfWjQ2JQSQL15KLh6D7xQhx0qgZlYZTYlcEiRuAMMw=", "owner": "nix-community", "repo": "nixpkgs.lib", "rev": "e1e11fdbb01113d85c7f41cada9d2847660e3902", "type": "github" },
      "original": { "owner": "nix-community", "repo": "nixpkgs.lib", "type": "github" }
    },
    "root": {
      "inputs": { "flake-parts": "flake-parts_3", "nix-colors": "nix-colors" }
    }
  },
  "root": "root",
  "version": 7
}"#;
        let config = FollowConfig {
            transitive_min: 2,
            ..FollowConfig::default()
        };

        let out = run_in_memory(flake, lock, &config).expect("run_in_memory");

        assert_eq!(out, None, "an alias-only group must not mint an input");
    }

    #[test]
    fn drop_overrides_under_redirects_drops_paths_under_declared_follows() {
        let inputs = make_input_map(vec![input_with_follows(
            "a",
            vec![(ap("mid"), Some(ap("mid")))],
        )]);
        let graph = FollowsGraph::from_declared(&inputs);
        let mut plan = FollowPlan {
            to_follow: vec![
                (ap("a.mid.nixpkgs"), ap("nixpkgs")),
                (ap("b.nixpkgs"), ap("nixpkgs")),
            ],
            ..FollowPlan::default()
        };

        drop_overrides_under_redirects(&graph, &mut plan);

        assert_eq!(plan.to_follow, vec![(ap("b.nixpkgs"), ap("nixpkgs"))]);
    }
}
