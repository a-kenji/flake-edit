//! [`FollowsGraph`] - typed, source-of-origin-tracked follows edges.
//!
//! Every graph-shaped follows operation lives here. Each edge knows whether
//! it came from `flake.nix` (declared) or from `flake.lock` (resolved);
//! paths are typed [`AttrPath`]s all the way down so structural equality
//! replaces the older quote-normalising string compare.
use std::collections::{HashMap, HashSet};

use crate::edit::InputMap;
use crate::error::FlakeEditError;
use crate::follows::AttrPath;
use crate::input::{Follows, Input, Range};
use crate::lock::FlakeLock;

/// Default upper bound on graph traversal depth. The per-emission cap
/// (`follow.max_depth` in config) is a separate, smaller knob.
pub const DEFAULT_MAX_DEPTH: usize = 64;

/// Where a [`FollowsGraph`] edge originated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EdgeOrigin {
    /// Edge declared explicitly in `flake.nix`.
    Declared {
        /// Source-text byte range of the declaring `inputs...follows = "..."`
        /// attrpath/value, when known. Empty range means "no source location
        /// available" (constructed in tests / via `from_lock` only).
        range: Range,
    },
    /// Edge discovered by walking `flake.lock`.
    Resolved {
        /// Lockfile node owning the parent (left) side of the edge.
        parent_node: String,
        /// Lockfile node the edge resolves to.
        target_node: String,
    },
}

/// One follows edge in the graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    /// Where the edge starts: e.g. `crane.nixpkgs` for an
    /// `inputs.crane.inputs.nixpkgs.follows` declaration.
    pub source: AttrPath,
    /// What the edge points at: the right-hand side of the `follows`.
    pub follows: AttrPath,
    /// Origin metadata; declared edges carry source ranges, resolved edges
    /// carry lockfile node names.
    pub origin: EdgeOrigin,
}

/// A declared follows whose lockfile resolution disagrees with what
/// `flake.nix` asked for. Produced by
/// [`FollowsGraph::stale_lock_declarations`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleLockDeclaration<'a> {
    /// The declared edge (carries the source-text range for diagnostics).
    pub declared: &'a Edge,
    /// What the lockfile resolves the same source path to. `None` means the
    /// lockfile has the path but no follows attached (the override was never
    /// applied).
    pub lock_target: Option<AttrPath>,
}

/// A detected cycle, expressed as the sequence of edges that close it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cycle {
    /// Edges in traversal order. The last edge's `follows` is structurally
    /// equal to the first edge's `source` (or a structural prefix of it).
    pub edges: Vec<Edge>,
}

/// A group of nested inputs that share a transitive follows target.
///
/// The grouping key is the canonical (alias-resolved) name of the nested
/// input; the value is the shared `target` plus every nested-path that
/// contributed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitiveGroup {
    /// Canonical name (post-alias) the group shares.
    pub canonical_name: String,
    /// The follows target every member resolves to (lockfile-side).
    pub target: AttrPath,
    /// All declared/resolved sources that share this target.
    pub members: Vec<AttrPath>,
}

#[derive(Debug, thiserror::Error)]
pub enum GraphError {
    #[error(transparent)]
    Lock(#[from] FlakeEditError),
}

/// Index of follows edges keyed by their `source` path, with construction
/// helpers for declared- / resolved-only / full-flake views.
#[derive(Debug, Clone)]
pub struct FollowsGraph {
    edges: HashMap<AttrPath, Vec<Edge>>,
    /// Every nested-input path observed in the lockfile, regardless of
    /// whether it carries a follows. Populated by [`Self::from_lock`] and
    /// [`Self::from_flake`]. Used by [`Self::stale_edges`] to ask "is the source
    /// path of this declared edge still represented in the resolved view?".
    resolved_universe: HashSet<AttrPath>,
    max_depth: usize,
}

impl Default for FollowsGraph {
    fn default() -> Self {
        FollowsGraph {
            edges: HashMap::new(),
            resolved_universe: HashSet::new(),
            max_depth: DEFAULT_MAX_DEPTH,
        }
    }
}

impl FollowsGraph {
    /// Build a graph from the declared `inputs = { ... }` block alone.
    ///
    /// Every [`Follows::Indirect`] in the input map becomes one
    /// [`EdgeOrigin::Declared`] edge with the `Input::range` byte range.
    pub fn from_declared(inputs: &InputMap) -> Self {
        let mut graph = FollowsGraph {
            max_depth: DEFAULT_MAX_DEPTH,
            ..FollowsGraph::default()
        };
        for input in inputs.values() {
            collect_declared_edges(input, &mut graph);
        }
        graph
    }

    /// Build a graph from the lockfile alone.
    ///
    /// Walks `flake.lock` recursively from the root, emitting one
    /// [`EdgeOrigin::Resolved`] edge per `inputs.X = ["a", "b", ...]` follows
    /// override, bounded by `DEFAULT_MAX_DEPTH`. Every nested-input path is
    /// recorded in `resolved_universe` regardless of whether it carries a
    /// follows.
    pub fn from_lock(lock: &FlakeLock) -> Result<Self, GraphError> {
        let mut graph = FollowsGraph {
            max_depth: DEFAULT_MAX_DEPTH,
            ..FollowsGraph::default()
        };
        for nested in lock.nested_inputs() {
            graph.resolved_universe.insert(nested.path.clone());
            if let Some(target) = nested.follows {
                graph.insert_edge(Edge {
                    source: nested.path.clone(),
                    follows: target,
                    origin: EdgeOrigin::Resolved {
                        parent_node: nested.path.first().as_str().to_string(),
                        target_node: nested.path.last().as_str().to_string(),
                    },
                });
            }
        }
        Ok(graph)
    }

    /// Build the follows graph for a flake: declared edges first, then
    /// resolved edges from the lockfile that aren't already covered by a
    /// declared edge with the same `source` path. Records every nested-input
    /// path observed in the lockfile in `resolved_universe`.
    pub fn from_flake(inputs: &InputMap, lock: &FlakeLock) -> Result<Self, GraphError> {
        let mut graph = FollowsGraph::from_declared(inputs);
        for nested in lock.nested_inputs() {
            graph.resolved_universe.insert(nested.path.clone());
            if let Some(target) = nested.follows {
                if graph.edges.contains_key(&nested.path) {
                    continue;
                }
                graph.insert_edge(Edge {
                    source: nested.path.clone(),
                    follows: target,
                    origin: EdgeOrigin::Resolved {
                        parent_node: nested.path.first().as_str().to_string(),
                        target_node: nested.path.last().as_str().to_string(),
                    },
                });
            }
        }
        Ok(graph)
    }

    /// Override the traversal depth bound. Default is [`DEFAULT_MAX_DEPTH`].
    #[must_use]
    pub fn with_max_depth(mut self, max: usize) -> Self {
        self.max_depth = max;
        self
    }

    /// Outgoing edges from `src` (empty slice if none).
    pub fn outgoing(&self, src: &AttrPath) -> &[Edge] {
        self.edges.get(src).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Iterator over every edge in deterministic order (lex by source).
    pub fn edges(&self) -> impl Iterator<Item = &Edge> {
        let mut keys: Vec<&AttrPath> = self.edges.keys().collect();
        keys.sort();
        keys.into_iter()
            .flat_map(|k| self.edges.get(k).unwrap().iter())
    }

    /// Cycles among declared edges only. Detects per-segment self-cycles
    /// (an edge whose source equals its follows). Multi-hop and
    /// lockfile-only cycle detection lives on
    /// [`Self::would_create_cycle`].
    pub fn cycles(&self) -> Vec<Cycle> {
        let mut found: Vec<Cycle> = Vec::new();
        let mut seen_keys: HashSet<(AttrPath, AttrPath)> = HashSet::new();
        let mut declared: Vec<&Edge> = self
            .edges()
            .filter(|e| matches!(e.origin, EdgeOrigin::Declared { .. }))
            .collect();
        declared.sort_by(|a, b| a.source.cmp(&b.source).then(a.follows.cmp(&b.follows)));
        for edge in declared {
            if !is_one_step_cycle(edge) {
                continue;
            }
            let key = (edge.source.clone(), edge.follows.clone());
            if seen_keys.insert(key) {
                found.push(Cycle {
                    edges: vec![edge.clone()],
                });
            }
        }
        found
    }

    /// Edges declared in `flake.nix` whose source path is no longer present
    /// in the lockfile's nested-input universe. A follows declaration whose
    /// nested input no longer exists in `flake.lock` should be dropped on
    /// the next auto-follow pass. Always returned in a deterministic (lex
    /// by source) order.
    pub fn stale_edges(&self) -> Vec<&Edge> {
        let mut declared: Vec<&Edge> = self
            .edges()
            .filter(|e| matches!(e.origin, EdgeOrigin::Declared { .. }))
            .filter(|e| !self.resolved_universe.contains(&e.source))
            .collect();
        declared.sort_by(|a, b| a.source.cmp(&b.source));
        declared
    }

    /// Declared follows whose target disagrees with the lockfile's resolution
    /// for the same source path.
    ///
    /// A returned entry means: `flake.nix` declares a follows for
    /// `entry.declared.source` pointing at `entry.declared.follows`, but the
    /// lock has either:
    ///
    /// - `lock_target = Some(other)` - a different resolved target, OR
    /// - `lock_target = None`        - no follows at all (the override was
    ///   never applied).
    ///
    /// In both cases the remediation is `nix flake lock`. Sources that the
    /// lockfile has not seen at all are reported by [`Self::stale_edges`]
    /// instead, not here.
    ///
    /// Returned in deterministic (lex by source) order.
    pub fn stale_lock_declarations<'a>(
        &'a self,
        lock: &FlakeLock,
    ) -> Vec<StaleLockDeclaration<'a>> {
        let lock_targets: HashMap<AttrPath, Option<AttrPath>> = lock
            .nested_inputs()
            .into_iter()
            .map(|n| (n.path, n.follows))
            .collect();

        let mut out: Vec<StaleLockDeclaration<'a>> = Vec::new();
        for edge in self.edges() {
            if !matches!(edge.origin, EdgeOrigin::Declared { .. }) {
                continue;
            }
            let Some(lock_target) = lock_targets.get(&edge.source) else {
                continue;
            };
            let diverges = match lock_target {
                Some(target) => target != &edge.follows,
                None => true,
            };
            if diverges {
                out.push(StaleLockDeclaration {
                    declared: edge,
                    lock_target: lock_target.clone(),
                });
            }
        }
        out.sort_by(|a, b| a.declared.source.cmp(&b.declared.source));
        out
    }

    /// Returns `true` when adding `proposed` to the graph would create a
    /// follows cycle.
    ///
    /// Multi-hop, origin-agnostic depth-first search. The walk starts at
    /// `proposed.follows` and chases outgoing edges in the existing graph;
    /// if it ever reaches `proposed.source`, the proposed edge would close
    /// a cycle. Three bug classes are explicitly covered (in addition to
    /// the trivial self-edge case):
    ///
    /// 1. **Dot-named ancestor.** Equality on [`AttrPath`] is structural,
    ///    so a participant named e.g. `"hls-1.10"` is compared by its
    ///    unquoted segment value rather than by URL-string prefix.
    /// 2. **Multi-hop chains.** A cycle of the shape
    ///    `A → B → C → ... → A` is found by traversing the chain.
    /// 3. **Lockfile-only cycles.** Edges discovered via
    ///    [`Self::from_lock`] / [`Self::merged`] carry
    ///    [`EdgeOrigin::Resolved`]; the DFS treats declared and resolved
    ///    edges identically, so a chain that closes only through the
    ///    lockfile is still reported.
    ///
    /// The traversal is bounded by [`Self::max_depth`] as a safety net for
    /// malformed graphs, and the standard DFS visited / on-stack sets keep
    /// pre-existing cycles in the graph from wedging the walk.
    pub fn would_create_cycle(&self, proposed: &Edge) -> bool {
        if is_one_step_cycle(proposed) {
            return true;
        }
        // Structural ancestor case: if the proposed `follows` target's leading
        // segment matches any ancestor segment of the source, the proposed
        // edge would point a nested input back at one of its own ancestors -
        // a degenerate self-reference Nix forbids. Catches the dot-named
        // ancestor case even when no other edges are present in the graph yet.
        let target_first = proposed.follows.first();
        let mut ancestor: Option<AttrPath> = proposed.source.parent();
        while let Some(a) = ancestor {
            if a.last() == target_first {
                return true;
            }
            ancestor = a.parent();
        }
        let mut visited: HashSet<AttrPath> = HashSet::new();
        let mut on_stack: HashSet<AttrPath> = HashSet::new();
        self.dfs_reaches(
            &proposed.follows,
            &proposed.source,
            0,
            &mut visited,
            &mut on_stack,
        )
    }

    fn dfs_reaches(
        &self,
        node: &AttrPath,
        target: &AttrPath,
        depth: usize,
        visited: &mut HashSet<AttrPath>,
        on_stack: &mut HashSet<AttrPath>,
    ) -> bool {
        if depth >= self.max_depth {
            return false;
        }
        if node == target {
            return true;
        }
        if visited.contains(node) {
            return false;
        }
        if on_stack.contains(node) {
            // Pre-existing cycle in the graph that doesn't pass through
            // `target`; skip it without claiming the proposed edge closes a
            // new one.
            return false;
        }
        // Also detect a target that lies inside the current node's
        // subtree - a cycle through structural ancestry, e.g. proposed
        // `c.a -> a` while traversing from `a` whose subtree contains
        // declared edges sourced at `a.x`, transitively reaching `c.a`.
        if node.is_prefix_of(target) {
            return true;
        }
        on_stack.insert(node.clone());
        // Expand both literal-source edges and edges whose source starts
        // with `node`. The second case captures the implicit
        // "parent depends on target" relation expressed by a declared
        // follows `parent.child -> target` in real flakes.
        for edge in self.expanded_outgoing(node) {
            if self.dfs_reaches(&edge.follows, target, depth + 1, visited, on_stack) {
                on_stack.remove(node);
                visited.insert(node.clone());
                return true;
            }
        }
        on_stack.remove(node);
        visited.insert(node.clone());
        false
    }

    /// All edges that semantically describe outgoing dependencies of `node`:
    /// edges whose source equals `node`, plus edges whose source has `node`
    /// as a strict prefix (i.e. `node`'s descendants in the source-path
    /// hierarchy). The latter encode the "parent depends on target"
    /// relation implicit in a declared follows
    /// `parent.child.follows = target`.
    fn expanded_outgoing(&self, node: &AttrPath) -> Vec<&Edge> {
        let mut out: Vec<&Edge> = Vec::new();
        for (source, edges) in &self.edges {
            if source == node || node.is_prefix_of(source) {
                out.extend(edges.iter());
            }
        }
        out
    }

    /// Group nested inputs by canonical (caller-supplied via `top` set
    /// membership) name, with deterministic ordering.
    ///
    /// The `top` set is the set of top-level input names already present in
    /// `flake.nix`; sources whose first segment is in `top` are skipped (no
    /// promotion would be possible).
    pub fn transitive_groups(&self, top: &HashSet<AttrPath>) -> Vec<TransitiveGroup> {
        let mut by_canonical: HashMap<String, HashMap<AttrPath, Vec<AttrPath>>> = HashMap::new();
        for edge in self.edges() {
            let nested_name = edge.source.last().as_str().to_string();
            let toplevel_only = AttrPath::parse(&nested_name).ok();
            if let Some(p) = &toplevel_only
                && top.contains(p)
            {
                continue;
            }
            by_canonical
                .entry(nested_name)
                .or_default()
                .entry(edge.follows.clone())
                .or_default()
                .push(edge.source.clone());
        }

        let mut groups: Vec<TransitiveGroup> = Vec::new();
        let mut canonical_names: Vec<String> = by_canonical.keys().cloned().collect();
        canonical_names.sort();
        for name in canonical_names {
            let buckets = by_canonical.remove(&name).unwrap();
            let mut bucket_keys: Vec<AttrPath> = buckets.keys().cloned().collect();
            bucket_keys.sort();
            for target in bucket_keys {
                let mut members = buckets.get(&target).cloned().unwrap_or_default();
                members.sort();
                groups.push(TransitiveGroup {
                    canonical_name: name.clone(),
                    target,
                    members,
                });
            }
        }
        groups
    }

    fn insert_edge(&mut self, edge: Edge) {
        self.edges
            .entry(edge.source.clone())
            .or_default()
            .push(edge);
    }
}

fn collect_declared_edges(input: &Input, graph: &mut FollowsGraph) {
    for follows in input.follows() {
        if let Follows::Indirect { path, target } = follows {
            let mut source = AttrPath::new(input.id().clone());
            for seg in path.segments() {
                source.push(seg.clone());
            }
            graph.insert_edge(Edge {
                source,
                follows: target.clone(),
                origin: EdgeOrigin::Declared {
                    range: input.range.clone(),
                },
            });
        }
    }
}

/// Per-segment self-cycle: source equals follows, structurally.
fn is_one_step_cycle(edge: &Edge) -> bool {
    edge.source == edge.follows
}

/// Whether the URL string of a top-level input is itself a follows reference
/// of the form `"<parent>/<rest>"`. Exposed so consumers with only a URL
/// string (no typed target) can still ask the question; URLs are stored
/// unquoted in memory.
pub fn is_follows_reference_to_parent(url: &str, parent: &str) -> bool {
    url.starts_with(&format!("{parent}/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::follows::Segment;
    use crate::input::Range;

    fn seg(s: &str) -> Segment {
        Segment::from_unquoted(s).unwrap()
    }

    fn path(s: &str) -> AttrPath {
        AttrPath::parse(s).unwrap()
    }

    fn declared_input(id: &str, follows: &[(&str, &str)]) -> Input {
        let mut input = Input::new(seg(id));
        for (parent, target) in follows {
            input.follows.push(Follows::Indirect {
                path: AttrPath::new(seg(parent)),
                target: path(target),
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

    fn declared_edge(source: &str, follows: &str) -> Edge {
        Edge {
            source: path(source),
            follows: path(follows),
            origin: EdgeOrigin::Declared {
                range: Range { start: 0, end: 0 },
            },
        }
    }

    #[test]
    fn from_declared_emits_one_edge_per_indirect() {
        let inputs = make_inputs(vec![declared_input(
            "crane",
            &[("nixpkgs", "nixpkgs"), ("flake-utils", "flake-utils")],
        )]);
        let g = FollowsGraph::from_declared(&inputs);
        let mut got: Vec<(String, String)> = g
            .edges()
            .map(|e| (e.source.to_string(), e.follows.to_string()))
            .collect();
        got.sort();
        assert_eq!(
            got,
            vec![
                ("crane.flake-utils".to_string(), "flake-utils".to_string()),
                ("crane.nixpkgs".to_string(), "nixpkgs".to_string()),
            ]
        );
    }

    #[test]
    fn from_declared_marks_origin_declared() {
        let inputs = make_inputs(vec![declared_input("crane", &[("nixpkgs", "nixpkgs")])]);
        let g = FollowsGraph::from_declared(&inputs);
        let edge = g.edges().next().unwrap();
        assert!(matches!(edge.origin, EdgeOrigin::Declared { .. }));
    }

    #[test]
    fn from_lock_picks_up_nested_follows() {
        let lock_text = r#"{
  "nodes": {
    "nixpkgs": {
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "abc", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "treefmt-nix": {
      "inputs": { "nixpkgs": ["nixpkgs"] },
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "def", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "root": {
      "inputs": { "nixpkgs": "nixpkgs", "treefmt-nix": "treefmt-nix" }
    }
  },
  "root": "root",
  "version": 7
}"#;
        let lock = FlakeLock::read_from_str(lock_text).unwrap();
        let g = FollowsGraph::from_lock(&lock).unwrap();
        let edges: Vec<&Edge> = g.edges().collect();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].source.to_string(), "treefmt-nix.nixpkgs");
        assert_eq!(edges[0].follows.to_string(), "nixpkgs");
        assert!(matches!(edges[0].origin, EdgeOrigin::Resolved { .. }));
    }

    #[test]
    fn outgoing_returns_only_matching_source() {
        let inputs = make_inputs(vec![
            declared_input("crane", &[("nixpkgs", "nixpkgs")]),
            declared_input("flake-utils", &[("nixpkgs", "nixpkgs")]),
        ]);
        let g = FollowsGraph::from_declared(&inputs);
        let crane_out = g.outgoing(&path("crane.nixpkgs"));
        assert_eq!(crane_out.len(), 1);
        assert_eq!(crane_out[0].follows.to_string(), "nixpkgs");
        assert!(g.outgoing(&path("nonexistent.nixpkgs")).is_empty());
    }

    #[test]
    fn stale_edges_flags_declared_without_resolved() {
        // Construct a merged graph where flake.nix declares
        // `home-manager.nixpkgs.follows`, but the lockfile no longer has any
        // nested input at that path.
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
        let lock = FlakeLock::read_from_str(lock_text).unwrap();
        let g = FollowsGraph::from_flake(&inputs, &lock).unwrap();
        let stale: Vec<&Edge> = g.stale_edges();
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].source.to_string(), "home-manager.nixpkgs");
    }

    #[test]
    fn transitive_groups_skips_when_canonical_already_top_level() {
        let inputs = make_inputs(vec![
            declared_input("crane", &[("nixpkgs", "nixpkgs")]),
            declared_input("flake-utils", &[("nixpkgs", "nixpkgs")]),
        ]);
        let g = FollowsGraph::from_declared(&inputs);
        let mut top: HashSet<AttrPath> = HashSet::new();
        top.insert(path("nixpkgs"));
        let groups = g.transitive_groups(&top);
        assert!(groups.is_empty());
    }

    #[test]
    fn transitive_groups_groups_shared_target() {
        let inputs = make_inputs(vec![
            declared_input("crane", &[("flake-parts", "flake-parts")]),
            declared_input("treefmt-nix", &[("flake-parts", "flake-parts")]),
        ]);
        let g = FollowsGraph::from_declared(&inputs);
        let top: HashSet<AttrPath> = HashSet::new();
        let groups = g.transitive_groups(&top);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].canonical_name, "flake-parts");
        assert_eq!(groups[0].target.to_string(), "flake-parts");
        let members: Vec<String> = groups[0].members.iter().map(|p| p.to_string()).collect();
        assert_eq!(
            members,
            vec!["crane.flake-parts", "treefmt-nix.flake-parts"]
        );
    }

    #[test]
    fn transitive_groups_sorted_by_canonical_then_target() {
        let inputs = make_inputs(vec![
            declared_input("a", &[("z", "z")]),
            declared_input("b", &[("z", "z")]),
            declared_input("c", &[("y", "y")]),
        ]);
        let g = FollowsGraph::from_declared(&inputs);
        let top: HashSet<AttrPath> = HashSet::new();
        let groups = g.transitive_groups(&top);
        let names: Vec<&str> = groups.iter().map(|g| g.canonical_name.as_str()).collect();
        assert_eq!(names, vec!["y", "z"]);
    }

    #[test]
    fn would_create_cycle_self_edge() {
        let g = FollowsGraph::default();
        let e = declared_edge("nixpkgs", "nixpkgs");
        assert!(g.would_create_cycle(&e));
    }

    #[test]
    fn would_create_cycle_dot_named_ancestor() {
        // Source `"hls-1.10".nixpkgs` with target `"hls-1.10"`: structural
        // equality on the dot-named segment must catch the ancestor cycle.
        let g = FollowsGraph::default();
        let e = Edge {
            source: AttrPath::parse("\"hls-1.10\".nixpkgs").unwrap(),
            follows: AttrPath::parse("\"hls-1.10\"").unwrap(),
            origin: EdgeOrigin::Declared {
                range: Range { start: 0, end: 0 },
            },
        };
        assert!(g.would_create_cycle(&e));
    }

    #[test]
    fn would_create_cycle_no_cycle_for_distinct_target() {
        let g = FollowsGraph::default();
        let e = declared_edge("crane.nixpkgs", "nixpkgs");
        assert!(!g.would_create_cycle(&e));
    }

    #[test]
    fn would_create_cycle_ignores_unrelated_ancestor() {
        let g = FollowsGraph::default();
        let e = declared_edge("a.b.c", "d");
        assert!(!g.would_create_cycle(&e));
    }

    /// Multi-hop cycle through declared edges only: `A → B`, `B → C`, propose
    /// `C → A`. The DFS must traverse the chain rather than relying on a
    /// per-ancestor check on `proposed`.
    #[test]
    fn would_create_cycle_multi_hop_declared() {
        let mut g = FollowsGraph::default();
        g.insert_edge(declared_edge("a", "b"));
        g.insert_edge(declared_edge("b", "c"));
        let proposed = declared_edge("c", "a");
        assert!(g.would_create_cycle(&proposed));
    }

    /// Multi-hop with a `"hls-1.10"` participant - typed equality via
    /// [`AttrPath`] must survive the embedded dot.
    #[test]
    fn would_create_cycle_multi_hop_dot_named() {
        let mut g = FollowsGraph::default();
        g.insert_edge(declared_edge("\"hls-1.10\"", "b"));
        g.insert_edge(declared_edge("b", "c"));
        let proposed = declared_edge("c", "\"hls-1.10\"");
        assert!(g.would_create_cycle(&proposed));
    }

    /// Lockfile-only cycle: a 2-hop chain that closes only through resolved
    /// edges. Origin-agnostic DFS must report it.
    #[test]
    fn would_create_cycle_lockfile_only() {
        let mut g = FollowsGraph::default();
        g.insert_edge(Edge {
            source: AttrPath::parse("treefmt-nix.nixpkgs").unwrap(),
            follows: AttrPath::parse("harmonia.treefmt-nix").unwrap(),
            origin: EdgeOrigin::Resolved {
                parent_node: "treefmt-nix".into(),
                target_node: "harmonia".into(),
            },
        });
        g.insert_edge(Edge {
            source: AttrPath::parse("harmonia.treefmt-nix").unwrap(),
            follows: AttrPath::parse("treefmt-nix").unwrap(),
            origin: EdgeOrigin::Resolved {
                parent_node: "harmonia".into(),
                target_node: "treefmt-nix".into(),
            },
        });
        // Propose `treefmt-nix → treefmt-nix.nixpkgs`: the chain reaches
        // `treefmt-nix` and closes back on the proposed source.
        let proposed = Edge {
            source: AttrPath::parse("treefmt-nix").unwrap(),
            follows: AttrPath::parse("treefmt-nix.nixpkgs").unwrap(),
            origin: EdgeOrigin::Declared {
                range: Range { start: 0, end: 0 },
            },
        };
        assert!(g.would_create_cycle(&proposed));
    }

    /// DFS still terminates and reports a cycle-closing proposal when the
    /// graph already contains an unrelated cycle.
    #[test]
    fn would_create_cycle_terminates_on_existing_cycle() {
        let mut g = FollowsGraph::default();
        g.insert_edge(declared_edge("x", "x"));
        g.insert_edge(declared_edge("a", "b"));
        g.insert_edge(declared_edge("b", "c"));
        let proposed = declared_edge("c", "a");
        assert!(g.would_create_cycle(&proposed));
    }

    /// `max_depth` bounds DFS so a malformed graph cannot wedge the resolver.
    #[test]
    fn would_create_cycle_bounded_by_max_depth() {
        let mut g = FollowsGraph::default().with_max_depth(2);
        g.insert_edge(declared_edge("a", "b"));
        g.insert_edge(declared_edge("b", "c"));
        g.insert_edge(declared_edge("c", "d"));
        let proposed = declared_edge("d", "a");
        assert!(!g.would_create_cycle(&proposed));
    }

    #[test]
    fn cycles_finds_self_referential_declared_edge() {
        let mut inputs = InputMap::new();
        let mut input = Input::new(seg("foo"));
        input.follows.push(Follows::Indirect {
            path: AttrPath::new(seg("foo")),
            target: AttrPath::parse("foo.foo").unwrap(),
        });
        input.range = Range { start: 1, end: 2 };
        inputs.insert("foo".into(), input);
        let g = FollowsGraph::from_declared(&inputs);
        let cycles = g.cycles();
        assert_eq!(cycles.len(), 1);
        assert_eq!(cycles[0].edges.len(), 1);
        assert_eq!(cycles[0].edges[0].source.to_string(), "foo.foo");
    }

    #[test]
    fn cycles_empty_for_acyclic_declared() {
        let inputs = make_inputs(vec![declared_input("crane", &[("nixpkgs", "nixpkgs")])]);
        let g = FollowsGraph::from_declared(&inputs);
        assert!(g.cycles().is_empty());
    }

    #[test]
    fn is_follows_reference_to_parent_url_prefix() {
        assert!(is_follows_reference_to_parent(
            "clan-core/treefmt-nix",
            "clan-core"
        ));
        assert!(!is_follows_reference_to_parent("github:nixos/nixpkgs", "x"));
        assert!(!is_follows_reference_to_parent(
            "clan-core-extended",
            "clan-core"
        ));
    }

    #[test]
    fn with_max_depth_overrides_default() {
        let g = FollowsGraph::default().with_max_depth(7);
        assert_eq!(g.max_depth, 7);
    }

    #[test]
    fn stale_lock_declarations_detects_target_mismatch() {
        let inputs = make_inputs(vec![declared_input("crane", &[("nixpkgs", "nixpkgs")])]);
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
        let lock = FlakeLock::read_from_str(lock_text).unwrap();
        let g = FollowsGraph::from_flake(&inputs, &lock).unwrap();
        let overridden = g.stale_lock_declarations(&lock);
        assert_eq!(overridden.len(), 1);
        assert_eq!(overridden[0].declared.source.to_string(), "crane.nixpkgs");
        assert_eq!(overridden[0].declared.follows.to_string(), "nixpkgs");
        assert_eq!(
            overridden[0].lock_target.as_ref().map(|p| p.to_string()),
            Some("nixpkgs_2".to_string())
        );
    }

    #[test]
    fn stale_lock_declarations_detects_missing_follows() {
        let inputs = make_inputs(vec![declared_input("crane", &[("nixpkgs", "nixpkgs")])]);
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
      "inputs": { "nixpkgs": "nixpkgs_2" },
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
        let lock = FlakeLock::read_from_str(lock_text).unwrap();
        let g = FollowsGraph::from_flake(&inputs, &lock).unwrap();
        let overridden = g.stale_lock_declarations(&lock);
        assert_eq!(overridden.len(), 1);
        assert_eq!(overridden[0].declared.source.to_string(), "crane.nixpkgs");
        assert!(overridden[0].lock_target.is_none());
    }

    #[test]
    fn stale_lock_declarations_quiet_when_in_sync() {
        let inputs = make_inputs(vec![declared_input("crane", &[("nixpkgs", "nixpkgs")])]);
        let lock_text = r#"{
  "nodes": {
    "nixpkgs": {
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "abc", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "crane": {
      "inputs": { "nixpkgs": ["nixpkgs"] },
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
        let lock = FlakeLock::read_from_str(lock_text).unwrap();
        let g = FollowsGraph::from_flake(&inputs, &lock).unwrap();
        assert!(g.stale_lock_declarations(&lock).is_empty());
    }

    #[test]
    fn stale_lock_declarations_orthogonal_to_stale() {
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
        let lock = FlakeLock::read_from_str(lock_text).unwrap();
        let g = FollowsGraph::from_flake(&inputs, &lock).unwrap();
        assert_eq!(g.stale_edges().len(), 1);
        assert!(g.stale_lock_declarations(&lock).is_empty());
    }

    #[test]
    fn merged_prefers_declared_over_resolved() {
        let inputs = make_inputs(vec![declared_input(
            "treefmt-nix",
            &[("nixpkgs", "nixpkgs")],
        )]);
        let lock_text = r#"{
  "nodes": {
    "nixpkgs": {
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "abc", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "treefmt-nix": {
      "inputs": { "nixpkgs": ["nixpkgs"] },
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "def", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "root": {
      "inputs": { "nixpkgs": "nixpkgs", "treefmt-nix": "treefmt-nix" }
    }
  },
  "root": "root",
  "version": 7
}"#;
        let lock = FlakeLock::read_from_str(lock_text).unwrap();
        let g = FollowsGraph::from_flake(&inputs, &lock).unwrap();
        let edges: Vec<&Edge> = g.outgoing(&path("treefmt-nix.nixpkgs")).iter().collect();
        assert_eq!(edges.len(), 1);
        assert!(matches!(edges[0].origin, EdgeOrigin::Declared { .. }));
    }
}
