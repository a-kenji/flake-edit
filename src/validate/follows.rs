//! Follows-graph lints.
//!
//! Each lint is a pure function over a [`FollowsGraph`] (or an [`InputMap`])
//! that returns [`ValidationError`]s. Lints take the graph by reference,
//! never mutate it, and never read from disk. [`super::validate_full`]
//! supplies the [`Location`]s.

use std::collections::{HashMap, HashSet};

use super::error::{Location, ValidationError};
use crate::edit::InputMap;
use crate::follows::{AttrPath, Edge, EdgeOrigin, FollowsGraph};
use crate::lock::FlakeLock;

/// Translate `edge`'s declared range into a 1-indexed [`Location`] via
/// `offset_to_location`. Resolved (lockfile-only) edges fall back to
/// `(line=1, column=1)` since they have no source-text range.
fn edge_location<F: Fn(usize) -> Location>(edge: &Edge, offset_to_location: &F) -> Location {
    match &edge.origin {
        EdgeOrigin::Declared { range } => offset_to_location(range.start),
        EdgeOrigin::Resolved { .. } => Location { line: 1, column: 1 },
    }
}

/// Lint cycles among declared and merged edges. Wraps each
/// [`crate::follows::Cycle`] from the graph into a
/// [`ValidationError::FollowsCycle`].
pub(crate) fn lint_follows_cycle<F: Fn(usize) -> Location>(
    graph: &FollowsGraph,
    offset_to_location: &F,
) -> Vec<ValidationError> {
    graph
        .cycles()
        .into_iter()
        .map(|cycle| {
            let location = cycle
                .edges
                .first()
                .map(|edge| edge_location(edge, offset_to_location))
                .unwrap_or(Location { line: 1, column: 1 });
            ValidationError::FollowsCycle { cycle, location }
        })
        .collect()
}

/// Lint declared follows whose source path has dropped out of the
/// lockfile's nested-input universe. Always [`super::Severity::Warning`].
///
/// `graph` must be a merged graph (declared plus resolved edges over the
/// lockfile's nested-input universe). The lint defers to
/// [`FollowsGraph::stale_edges`] to compare declared sources against the
/// resolved set.
pub(crate) fn lint_follows_stale<F: Fn(usize) -> Location>(
    graph: &FollowsGraph,
    offset_to_location: &F,
) -> Vec<ValidationError> {
    graph
        .stale_edges()
        .into_iter()
        .map(|edge| ValidationError::FollowsStale {
            edge: edge.clone(),
            location: edge_location(edge, offset_to_location),
        })
        .collect()
}

/// Lint declared follows whose lockfile resolution disagrees with what
/// `flake.nix` asks for. Wraps each [`crate::follows::StaleLockDeclaration`]
/// into a [`ValidationError::FollowsStaleLock`].
///
/// Always [`super::Severity::Warning`]. The remediation is `nix flake lock`.
pub(crate) fn lint_follows_stale_lock<F: Fn(usize) -> Location>(
    graph: &FollowsGraph,
    lock: &FlakeLock,
    offset_to_location: &F,
) -> Vec<ValidationError> {
    graph
        .stale_lock_declarations(lock)
        .into_iter()
        .map(|item| ValidationError::FollowsStaleLock {
            source: item.declared.source.clone(),
            declared_target: item.declared.follows.clone(),
            lock_target: item.lock_target,
            location: edge_location(item.declared, offset_to_location),
        })
        .collect()
}

/// Lint declared follows targets that fail to resolve to a top-level input.
///
/// `top_level` is the set of top-level input names from `flake.nix`, as
/// produced by [`top_level_names`]. Only declared edges are inspected.
/// Resolved edges reference real lockfile nodes by construction.
pub(crate) fn lint_follows_target_not_toplevel<F: Fn(usize) -> Location>(
    graph: &FollowsGraph,
    top_level: &HashSet<String>,
    offset_to_location: &F,
) -> Vec<ValidationError> {
    let mut out = Vec::new();
    for edge in graph.declared_edges() {
        let target_root = edge.follows.first().as_str();
        if !top_level.contains(target_root) {
            out.push(ValidationError::FollowsTargetNotToplevel {
                edge: edge.clone(),
                location: edge_location(edge, offset_to_location),
            });
        }
    }
    out
}

/// Lint declared follows that share a source path but disagree on the
/// target. Emits one [`ValidationError::FollowsContradiction`] per source
/// path with more than one distinct target among declared edges.
pub(crate) fn lint_follows_contradiction<F: Fn(usize) -> Location>(
    graph: &FollowsGraph,
    offset_to_location: &F,
) -> Vec<ValidationError> {
    let mut by_source: HashMap<AttrPath, Vec<&Edge>> = HashMap::new();
    for edge in graph.declared_edges() {
        by_source.entry(edge.source.clone()).or_default().push(edge);
    }
    let mut sources: Vec<&AttrPath> = by_source.keys().collect();
    sources.sort();
    let mut out = Vec::new();
    for source in sources {
        let edges = &by_source[source];
        let mut targets: Vec<&AttrPath> = edges.iter().map(|e| &e.follows).collect();
        targets.sort();
        targets.dedup();
        if targets.len() <= 1 {
            continue;
        }
        let cloned: Vec<Edge> = edges.iter().map(|&e| e.clone()).collect();
        let location = cloned
            .first()
            .map(|e| edge_location(e, offset_to_location))
            .unwrap_or(Location { line: 1, column: 1 });
        out.push(ValidationError::FollowsContradiction {
            edges: cloned,
            location,
        });
    }
    out
}

/// Lint declared follows whose source path is longer than `max_depth`
/// segments. Exceeding the bound means the path is deeper than the resolver
/// is willing to chase.
pub(crate) fn lint_follows_depth_exceeded<F: Fn(usize) -> Location>(
    graph: &FollowsGraph,
    max_depth: usize,
    offset_to_location: &F,
) -> Vec<ValidationError> {
    let mut out = Vec::new();
    for edge in graph.declared_edges() {
        let depth = edge.source.len();
        if depth > max_depth {
            out.push(ValidationError::FollowsDepthExceeded {
                edge: edge.clone(),
                depth,
                max_depth,
                location: edge_location(edge, offset_to_location),
            });
        }
    }
    out
}

/// Top-level input names from an [`InputMap`], shaped for
/// [`lint_follows_target_not_toplevel`].
pub(crate) fn top_level_names(inputs: &InputMap) -> HashSet<String> {
    inputs.keys().cloned().collect()
}

/// Build the follows graph from declared `flake.nix` edges, plus resolved
/// edges from `lock` when supplied. With `lock = None` only declared edges
/// are present and [`FollowsGraph::stale_edges`] reports nothing.
pub(crate) fn build_graph(
    inputs: &InputMap,
    lock: Option<&FlakeLock>,
    max_depth: usize,
) -> FollowsGraph {
    let graph = match lock {
        Some(lock) => FollowsGraph::from_flake(inputs, lock),
        None => FollowsGraph::from_declared(inputs),
    };
    graph.with_max_depth(max_depth)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::InputMap;
    use crate::follows::{AttrPath, Segment};
    use crate::input::{Follows, Input, Range};

    fn seg(s: &str) -> Segment {
        Segment::from_unquoted(s).unwrap()
    }

    fn path(s: &str) -> AttrPath {
        AttrPath::parse(s).unwrap()
    }

    fn loc_id() -> impl Fn(usize) -> Location {
        |_| Location { line: 1, column: 1 }
    }

    fn declared_input(id: &str, follows: &[(&str, &str)]) -> Input {
        let mut input = Input::new(seg(id));
        for (parent, target) in follows {
            input.follows.push(Follows::Indirect {
                path: AttrPath::new(seg(parent)),
                target: Some(path(target)),
            });
        }
        input.range = Range { start: 1, end: 2 };
        input
    }

    fn make_inputs(items: Vec<Input>) -> InputMap {
        let mut map = InputMap::new();
        for input in items {
            map.insert(input.id().as_str().to_string(), input);
        }
        map
    }

    #[test]
    fn cycle_lint_lifts_self_loop() {
        let inputs = make_inputs(vec![declared_input("foo", &[("foo", "foo.foo")])]);
        let graph = FollowsGraph::from_declared(&inputs);
        let errs = lint_follows_cycle(&graph, &loc_id());
        assert_eq!(errs.len(), 1);
        assert!(matches!(errs[0], ValidationError::FollowsCycle { .. }));
    }

    #[test]
    fn stale_lint_returns_empty_without_lock() {
        // Without a lock the resolved universe is empty, so every declared
        // source registers as stale. The lock-aware lint only runs when the
        // caller supplies a lock. The surrounding integration filters
        // accordingly.
        let inputs = make_inputs(vec![declared_input(
            "home-manager",
            &[("nixpkgs", "nixpkgs")],
        )]);
        let graph = build_graph(&inputs, None, 64);
        let stale = lint_follows_stale(&graph, &loc_id());
        assert_eq!(stale.len(), 1);
    }

    #[test]
    fn stale_lint_with_lock_only_fires_for_missing_source() {
        let inputs = make_inputs(vec![declared_input(
            "home-manager",
            &[("nixpkgs", "nixpkgs")],
        )]);
        let lock_text = r#"{
  "nodes": {
    "nixpkgs": {
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "abc", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "home-manager": {
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "ddd", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "root": {
      "inputs": { "nixpkgs": "nixpkgs", "home-manager": "home-manager" }
    }
  },
  "root": "root",
  "version": 7
}"#;
        let lock = crate::lock::FlakeLock::read_from_str(lock_text).unwrap();
        let graph = build_graph(&inputs, Some(&lock), 64);
        let stale = lint_follows_stale(&graph, &loc_id());
        assert_eq!(stale.len(), 1);
        assert!(matches!(stale[0], ValidationError::FollowsStale { .. }));
        assert_eq!(stale[0].severity(), super::super::Severity::Warning);
    }

    #[test]
    fn lints_emit_stale_lock_when_targets_diverge() {
        let inputs = make_inputs(vec![
            declared_input("crane", &[("nixpkgs", "nixpkgs")]),
            declared_input("nixpkgs", &[]),
        ]);
        let lock_text = r#"{
  "nodes": {
    "nixpkgs": {
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "abc", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "nixpkgs_2": {
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "def", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "crane": {
      "inputs": { "nixpkgs": ["nixpkgs_2"] },
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "ggg", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "root": {
      "inputs": { "nixpkgs": "nixpkgs", "crane": "crane" }
    }
  },
  "root": "root",
  "version": 7
}"#;
        let lock = crate::lock::FlakeLock::read_from_str(lock_text).unwrap();
        let graph = build_graph(&inputs, Some(&lock), 64);
        let errs = lint_follows_stale_lock(&graph, &lock, &loc_id());
        assert_eq!(errs.len(), 1);
        match &errs[0] {
            ValidationError::FollowsStaleLock {
                source,
                declared_target,
                lock_target,
                location,
            } => {
                assert_eq!(source.to_string(), "crane.nixpkgs");
                assert_eq!(declared_target.to_string(), "nixpkgs");
                assert_eq!(
                    lock_target.as_ref().map(|p| p.to_string()),
                    Some("nixpkgs_2".to_string())
                );
                assert_eq!(*location, Location { line: 1, column: 1 });
            }
            other => panic!("expected FollowsStaleLock, got {other:?}"),
        }
        assert_eq!(errs[0].severity(), super::super::Severity::Warning);
    }

    #[test]
    fn target_not_toplevel_flags_unknown_target() {
        let inputs = make_inputs(vec![
            declared_input("home-manager", &[("nixpkgs", "does-not-exist")]),
            declared_input("nixpkgs", &[]),
        ]);
        let graph = FollowsGraph::from_declared(&inputs);
        let top_level = top_level_names(&inputs);
        let errs = lint_follows_target_not_toplevel(&graph, &top_level, &loc_id());
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            errs[0],
            ValidationError::FollowsTargetNotToplevel { .. }
        ));
    }

    #[test]
    fn target_not_toplevel_passes_for_known_target() {
        let inputs = make_inputs(vec![
            declared_input("home-manager", &[("nixpkgs", "nixpkgs")]),
            declared_input("nixpkgs", &[]),
        ]);
        let graph = FollowsGraph::from_declared(&inputs);
        let top_level = top_level_names(&inputs);
        let errs = lint_follows_target_not_toplevel(&graph, &top_level, &loc_id());
        assert!(errs.is_empty());
    }

    #[test]
    fn contradiction_flags_two_distinct_targets_for_same_source() {
        // Mergeable `inputs = { ... }` attrsets land here alongside the
        // duplicate-attribute lint. This lint surfaces the graph-level
        // semantics.
        let synthetic = synthetic_graph_with_contradiction();
        let errs = lint_follows_contradiction(&synthetic, &loc_id());
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            errs[0],
            ValidationError::FollowsContradiction { .. }
        ));
    }

    /// Build a [`FollowsGraph`] with two declared edges that share a source
    /// but disagree on the target, by feeding [`FollowsGraph::from_declared`]
    /// an [`InputMap`] with two `Follows` entries on the same input.
    fn synthetic_graph_with_contradiction() -> FollowsGraph {
        let mut inputs = InputMap::new();
        let mut a = Input::new(seg("a"));
        a.follows.push(Follows::Indirect {
            path: AttrPath::new(seg("x")),
            target: Some(path("y")),
        });
        a.follows.push(Follows::Indirect {
            path: AttrPath::new(seg("x")),
            target: Some(path("z")),
        });
        a.range = Range { start: 1, end: 2 };
        inputs.insert("a".to_string(), a);
        FollowsGraph::from_declared(&inputs)
    }

    #[test]
    fn contradiction_passes_for_consistent_target() {
        let inputs = make_inputs(vec![declared_input("a", &[("x", "y")])]);
        let graph = FollowsGraph::from_declared(&inputs);
        let errs = lint_follows_contradiction(&graph, &loc_id());
        assert!(errs.is_empty());
    }

    #[test]
    fn depth_exceeded_flags_long_source_path() {
        // Source path `a.b.c` (depth 3) tripped against max_depth = 1.
        let mut inputs = InputMap::new();
        let mut a = Input::new(seg("a"));
        a.follows.push(Follows::Indirect {
            path: AttrPath::parse("b.c").unwrap(),
            target: Some(path("d")),
        });
        a.range = Range { start: 1, end: 2 };
        inputs.insert("a".to_string(), a);
        let graph = FollowsGraph::from_declared(&inputs);
        let errs = lint_follows_depth_exceeded(&graph, 1, &loc_id());
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            errs[0],
            ValidationError::FollowsDepthExceeded { .. }
        ));
    }

    #[test]
    fn depth_exceeded_passes_when_within_bound() {
        let inputs = make_inputs(vec![declared_input("a", &[("b", "c")])]);
        let graph = FollowsGraph::from_declared(&inputs);
        let errs = lint_follows_depth_exceeded(&graph, 4, &loc_id());
        assert!(errs.is_empty());
    }
}
