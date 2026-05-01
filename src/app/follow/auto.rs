//! Auto-deduplication of nested follows.
//!
//! Three structs thread through the pipeline:
//!
//! - [`AnalysisCtx`] - immutable read-only inputs (graph, config, bounds).
//! - [`FollowPlan`]  - mutable accumulator threaded through collection.
//! - [`AppliedPlan`] - outcomes of the apply step, consumed by render.

use std::collections::{HashMap, HashSet};

use crate::change::{Change, ChangeId};
use crate::config::FollowConfig;
use crate::edit::FlakeEdit;
use crate::follows::{
    AttrPath, Edge, EdgeOrigin, FollowsGraph, Segment, is_follows_reference_to_parent,
};
use crate::input::Range;
use crate::lock::FlakeLock;
use crate::validate;

use super::super::commands::{self, CommandError, FollowContext, Result};
use super::super::editor::Editor;
use super::super::state::AppState;

/// Public entry point for `flake-edit follow` on a single (in-memory) flake.
pub fn run(editor: &Editor, flake_edit: &mut FlakeEdit, state: &AppState) -> Result<()> {
    run_impl(editor, flake_edit, state, false)
}

/// Public entry point for batch mode (`flake-edit follow [PATHS...]`).
///
/// Each file is processed independently with its own Editor/AppState.
/// Errors are collected and reported at the end, but processing continues
/// for all files. Returns the first error if any file failed.
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
        for (path, err) in &errors {
            eprintln!("Error processing {}: {}", path.display(), err);
        }
        Err(errors.into_iter().next().unwrap().1)
    }
}

/// Read-only inputs threaded through analysis.
struct AnalysisCtx<'a> {
    ctx: &'a FollowContext,
    /// Lock-augmented graph: union of edges declared in `flake.nix` and edges
    /// the lockfile resolved. Used for cycle detection and stale-edge queries
    /// that need the fully-resolved view.
    graph: &'a FollowsGraph,
    /// Edges already written in `flake.nix` source. Distinct from `graph`
    /// because a follows that exists only in the lockfile (declared by an
    /// upstream flake) is still a candidate to write into the user's
    /// `flake.nix`.
    existing_follows: &'a HashSet<AttrPath>,
    follow_config: &'a FollowConfig,
    /// Maximum depth of follows declarations to write. `1` (default) writes
    /// depth-1 only; `2` or higher also writes grandchild and deeper.
    max_depth: usize,
    transitive_min: usize,
}

/// Mutable plan accumulated across the collection functions.
///
/// The four output buckets (`to_follow`, `to_unfollow`, `toplevel_follows`,
/// `toplevel_adds`) are what [`apply_plan`] consumes. `seen_nested` is a
/// dedup guard threaded through
/// [`collect_direct_candidates`],
/// [`collect_transitive_groups`],
/// [`collect_direct_groups`], and
/// [`emit_promotions`]; it prevents the same nested path from being scheduled
/// twice when both the direct-name match and a transitive group claim it.
/// It is not an applied output, hence excluded from [`FollowPlan::has_pending`].
#[derive(Default)]
struct FollowPlan {
    to_follow: Vec<(AttrPath, String)>,
    to_unfollow: Vec<AttrPath>,
    toplevel_follows: Vec<(AttrPath, String)>,
    toplevel_adds: Vec<(String, String)>,
    seen_nested: HashSet<AttrPath>,
}

impl FollowPlan {
    /// True if at least one applicable change was scheduled. `seen_nested`
    /// is intentionally ignored: it is dedup state, not pending output.
    fn has_pending(&self) -> bool {
        !self.to_follow.is_empty()
            || !self.to_unfollow.is_empty()
            || !self.toplevel_follows.is_empty()
            || !self.toplevel_adds.is_empty()
    }
}

/// What [`apply_plan`] actually committed to the working text.
#[derive(Default)]
struct AppliedPlan {
    /// Working text after every successful change. Equal to the original
    /// when no per-step change applied.
    current_text: String,
    /// `(source_path, target)` follows that were successfully applied,
    /// for the success summary.
    applied_follows: Vec<(String, String)>,
    /// Stale follows declarations that were removed.
    unfollowed: Vec<String>,
    /// Validation warnings observed across speculative applications, in
    /// arrival order. Caller deduplicates for display.
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
    // stale?" - that needs the full resolved view. `existing_follows`
    // (declared-only) answers "is this nested input already followed in
    // flake.nix?" - we must not suppress emission for an edge that the
    // lock resolved but the source never declared.
    let graph = match commands::load_flake_lock(state) {
        Ok(lock) => FollowsGraph::from_flake(&ctx.inputs, &lock)
            .unwrap_or_else(|_| FollowsGraph::from_declared(&ctx.inputs)),
        Err(_) => FollowsGraph::from_declared(&ctx.inputs),
    };

    // Filter the merged graph by EdgeOrigin::Declared rather than rebuilding
    // a separate `from_declared` graph; the declared subset is already
    // present in `graph`.
    let existing_follows: HashSet<AttrPath> = graph
        .edges()
        .filter(|e| matches!(e.origin, EdgeOrigin::Declared { .. }))
        .map(|e| e.source.clone())
        .collect();

    let follow_config = &state.config.follow;
    let transitive_min = follow_config.transitive_min();
    let max_depth = follow_config.max_depth.max(1);

    let ax = AnalysisCtx {
        ctx: &ctx,
        graph: &graph,
        existing_follows: &existing_follows,
        follow_config,
        max_depth,
        transitive_min,
    };

    let mut plan = FollowPlan {
        to_unfollow: graph
            .stale_edges()
            .into_iter()
            .map(|e| e.source.clone())
            .collect(),
        ..FollowPlan::default()
    };

    collect_direct_candidates(&ax, &mut plan);

    if transitive_min > 0 {
        let transitive_groups = collect_transitive_groups(&ax, &plan);
        let direct_groups = collect_direct_groups(&ax, &plan);
        emit_promotions(&ax, editor, transitive_groups, direct_groups, &mut plan);
    }

    if !plan.has_pending() {
        if !quiet {
            println!("All inputs are already deduplicated.");
        }
        return Ok(());
    }

    let applied = apply_plan(editor, state, &plan)?;
    render_summary(editor, state, &applied, quiet)
}

/// Path-shape filter shared by every collection function: depth-bounded, with
/// at least a parent segment.
///
/// Path length encodes depth: `parent.nested` is depth 1 (length 2);
/// `parent.middle.grandchild` is depth 2 (length 3). The bound
/// `len() <= max_depth + 1` admits exactly the configured depth.
fn within_depth(path: &AttrPath, max_depth: usize) -> bool {
    path.len() >= 2 && path.len() <= max_depth + 1
}

fn collect_direct_candidates(ax: &AnalysisCtx<'_>, plan: &mut FollowPlan) {
    for nested in ax.ctx.nested_inputs.iter() {
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
            .ctx
            .top_level_inputs
            .iter()
            .find(|top| ax.follow_config.can_follow(nested_name, top))
        else {
            continue;
        };

        if let Some(target_input) = ax.ctx.inputs.get(target.as_str())
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

        // Multi-hop / lockfile-only cycle detection. The URL-prefix check
        // above only catches the immediate-parent case; the merged graph
        // DFS catches the rest.
        if let Ok(target_path) = AttrPath::parse(target) {
            let proposed = Edge {
                source: nested.path.clone(),
                follows: target_path,
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
        }

        plan.seen_nested.insert(nested.path.clone());
        plan.to_follow.push((nested.path.clone(), target.clone()));
    }
}

fn collect_transitive_groups(
    ax: &AnalysisCtx<'_>,
    plan: &FollowPlan,
) -> HashMap<String, HashMap<AttrPath, Vec<AttrPath>>> {
    let mut groups: HashMap<String, HashMap<AttrPath, Vec<AttrPath>>> = HashMap::new();

    for nested in ax.ctx.nested_inputs.iter() {
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
        // Already handled by [`collect_direct_candidates`].
        if ax
            .ctx
            .top_level_inputs
            .iter()
            .any(|top| ax.follow_config.can_follow(nested_name, top))
        {
            continue;
        }

        let Some(transitive_target) = nested.follows.as_ref() else {
            continue;
        };
        // Only consider transitive follows (path with a parent segment).
        if transitive_target.len() < 2 {
            continue;
        }
        // Avoid self-follow situations.
        if transitive_target.last().as_str() == nested_name {
            continue;
        }

        let top_level_name = ax
            .follow_config
            .resolve_alias(nested_name)
            .unwrap_or(nested_name)
            .to_string();
        if ax.ctx.top_level_inputs.contains(&top_level_name) {
            continue;
        }

        if let Some(target_input) = ax.ctx.inputs.get(transitive_target.first().as_str())
            && is_follows_reference_to_parent(target_input.url(), parent)
        {
            continue;
        }

        // Multi-hop / lockfile-only cycle detection: mirror of the
        // direct-candidate filter, applied to transitive promotions.
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

    for nested in ax.ctx.nested_inputs.iter() {
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
            .ctx
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
        if ax.ctx.top_level_inputs.contains(&canonical_name) {
            continue;
        }

        groups
            .entry(canonical_name)
            .or_default()
            .push((nested.path.clone(), nested.url.clone()));
    }

    groups
}

/// Turn grouped candidates into concrete top-level follows and top-level
/// adds, and back-fill `plan.to_follow` with the per-nested follows that
/// hang off them.
fn emit_promotions(
    ax: &AnalysisCtx<'_>,
    editor: &Editor,
    transitive_groups: HashMap<String, HashMap<AttrPath, Vec<AttrPath>>>,
    direct_groups: HashMap<String, Vec<(AttrPath, Option<String>)>>,
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
        let follow_target = target_path.to_flake_follows_string();

        if follow_target == top_name {
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
            .push((AttrPath::new(top_seg), follow_target));

        for path in paths {
            if plan.seen_nested.insert(path.clone()) {
                plan.to_follow.push((path, top_name.clone()));
            }
        }
    }

    // Promote each direct-reference group: add a new top-level input with
    // the URL from one of the nested references, then have all sibling
    // paths follow it. Only promote if at least one follows can actually
    // be applied.
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
            FlakeEdit::from_text(&editor.text())
                .ok()
                .and_then(|mut fe| fe.apply_change(change).ok())
                .and_then(|outcome| outcome.text)
                .is_some()
        });
        if !can_follow {
            continue;
        }

        plan.toplevel_adds.push((canonical_name.clone(), url));

        for (path, _) in &entries {
            if plan.seen_nested.insert(path.clone()) {
                plan.to_follow.push((path.clone(), canonical_name.clone()));
            }
        }
    }
}

/// Apply each scheduled change against a working text buffer, validating
/// between steps.
///
/// Cycle-check staleness note: the planner consulted `ax.graph` (built
/// once from the original `flake.nix` plus lockfile) for `would_create_cycle`
/// in [`collect_direct_candidates`] and [`collect_transitive_groups`].
/// Speculative applies here mutate the working text but do not refresh that
/// planner-side graph. Per-step validation via [`validate::validate_full`]
/// rebuilds a graph from the post-change `temp.curr_list()` and runs the
/// cycle lint, so any cycle introduced by the in-progress batch surfaces
/// here as an Error and the offending change is skipped.
fn apply_plan(editor: &Editor, state: &AppState, plan: &FollowPlan) -> Result<AppliedPlan> {
    let mut current_text = editor.text();
    let batch_lock: Option<FlakeLock> = commands::load_flake_lock(state).ok();
    let mut warnings: Vec<validate::ValidationError> = Vec::new();

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
                    warnings.extend(outcome.warnings);
                    let validation = validate::validate_full(
                        &resulting_text,
                        temp.curr_list(),
                        batch_lock.as_ref(),
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

    let mut follow_changes: Vec<(AttrPath, String)> = plan.toplevel_follows.clone();
    follow_changes.extend(plan.to_follow.iter().cloned());

    let mut applied_follows: Vec<(String, String)> = Vec::new();

    for (input_path, target) in &follow_changes {
        let target_attr = match AttrPath::parse(target) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("Invalid follows target `{target}`: {e}");
                eprintln!("Invalid follows target `{target}`: {e}");
                continue;
            }
        };
        let change = Change::Follows {
            input: ChangeId::new(input_path.clone()),
            target: target_attr,
        };

        let mut temp = FlakeEdit::from_text(&current_text).map_err(CommandError::FlakeEdit)?;
        match temp.apply_change(change) {
            Ok(outcome) => match outcome.text {
                Some(resulting_text) => {
                    if resulting_text == current_text {
                        continue;
                    }
                    warnings.extend(outcome.warnings);
                    let validation = validate::validate_full(
                        &resulting_text,
                        temp.curr_list(),
                        batch_lock.as_ref(),
                    );
                    if validation.is_ok() {
                        warnings.extend(validation.warnings);
                        current_text = resulting_text;
                        applied_follows.push((input_path.to_string(), target.clone()));
                    } else {
                        for err in validation.errors {
                            eprintln!("Error applying follows for {}: {}", input_path, err);
                        }
                    }
                }
                None => eprintln!("Could not create follows for {}", input_path),
            },
            Err(e) => eprintln!("Error applying follows for {}: {}", input_path, e),
        }
    }

    let mut unfollowed: Vec<String> = Vec::new();

    for nested_path in &plan.to_unfollow {
        let change = Change::Remove {
            ids: vec![ChangeId::new(nested_path.clone())],
        };

        let mut temp = FlakeEdit::from_text(&current_text).map_err(CommandError::FlakeEdit)?;
        match temp.apply_change(change) {
            Ok(outcome) => {
                if let Some(resulting_text) = outcome.text {
                    warnings.extend(outcome.warnings);
                    let validation = validate::validate_full(
                        &resulting_text,
                        temp.curr_list(),
                        batch_lock.as_ref(),
                    );
                    if validation.is_ok() {
                        warnings.extend(validation.warnings);
                        current_text = resulting_text;
                        unfollowed.push(nested_path.to_string());
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

/// Stable identity for a follows-related warning, ignoring source-text
/// location. Two warnings with the same lint kind and key fields collapse
/// to one in the auto-follow output even if their reported lines differ
/// across iterations of `validate_full`.
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
