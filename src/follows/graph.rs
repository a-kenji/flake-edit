//! [`FollowsGraph`]: follows edges with origin tracking.
//!
//! Each [`Edge`] records whether it came from `flake.nix`
//! ([`EdgeOrigin::Declared`]) or `flake.lock` ([`EdgeOrigin::Resolved`]).
//! Paths are typed [`AttrPath`]s, so equality is structural.
use std::collections::{HashMap, HashSet};

use crate::edit::InputMap;
use crate::follows::{AttrPath, Segment};
use crate::input::{Follows, Input, Range};
use crate::lock::FlakeLock;

/// Default upper bound on graph traversal depth. The per-emission cap
/// (`follow.max_depth` in config) is a separate, smaller knob.
pub const DEFAULT_MAX_DEPTH: usize = 64;

/// Where an [`Edge`] originated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EdgeOrigin {
    /// Edge declared explicitly in `flake.nix`.
    Declared {
        /// Source-text byte range of the declaring `inputs...follows = "..."`
        /// attrpath/value, when known. An empty range means no source
        /// location is available (typical in tests and [`FollowsGraph::from_lock`]).
        range: Range,
    },
    /// Edge discovered by walking `flake.lock`.
    Resolved {
        /// Lockfile node owning the parent (left) side of the edge.
        parent_node: Segment,
        /// Lockfile node the edge resolves to.
        target_node: Segment,
    },
}

/// One follows edge in the graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    /// Where the edge starts, e.g. `crane.nixpkgs` for an
    /// `inputs.crane.inputs.nixpkgs.follows` declaration.
    pub source: AttrPath,
    /// Right-hand side of the `follows`.
    pub follows: AttrPath,
    /// Origin metadata: declared edges carry source ranges, resolved edges
    /// carry lockfile node names.
    pub origin: EdgeOrigin,
}

/// A declared follows whose lockfile resolution disagrees with `flake.nix`.
/// Produced by [`FollowsGraph::stale_lock_declarations`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleLockDeclaration<'a> {
    /// The declared edge, carrying its source-text range for diagnostics.
    pub declared: &'a Edge,
    /// What the lockfile resolves the same source path to. `None` means the
    /// lockfile has the path but no follows attached (override never applied).
    pub lock_target: Option<AttrPath>,
}

/// A detected cycle, as the sequence of edges that close it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cycle {
    /// Edges in traversal order. The last edge's `follows` equals the first
    /// edge's `source`, or is a structural prefix of it.
    pub edges: Vec<Edge>,
}

/// A group of nested inputs sharing a transitive follows target. Keyed by
/// the canonical (alias-resolved) name of the nested input. The value is the
/// shared `target` plus every contributing nested path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitiveGroup {
    /// Canonical (post-alias) name the group shares.
    pub canonical_name: String,
    /// Lockfile-side follows target every member resolves to.
    pub target: AttrPath,
    /// All declared or resolved sources that share this target.
    pub members: Vec<AttrPath>,
}

/// Index of follows [`Edge`]s keyed by their `source` path. Construct with
/// [`Self::from_declared`], [`Self::from_lock`], or [`Self::from_flake`] for
/// declared-only, resolved-only, or merged views respectively.
#[derive(Debug, Clone)]
pub struct FollowsGraph {
    edges: HashMap<AttrPath, Vec<Edge>>,
    /// Every nested-input path observed in the lockfile, including those
    /// without a follows. Populated by [`Self::from_lock`] and
    /// [`Self::from_flake`]. Consulted by [`Self::stale_edges`] to ask
    /// whether a declared edge's source path is still represented.
    resolved_universe: HashSet<AttrPath>,
    /// Source paths declared as `follows = ""` in `flake.nix`. These do
    /// not produce edges (no resolved target), but they still encode user
    /// intent: the auto-deduplicator must treat them as already-handled
    /// so it does not retarget them.
    declared_nulled_sources: HashSet<AttrPath>,
    max_depth: usize,
}

impl Default for FollowsGraph {
    fn default() -> Self {
        FollowsGraph {
            edges: HashMap::new(),
            resolved_universe: HashSet::new(),
            declared_nulled_sources: HashSet::new(),
            max_depth: DEFAULT_MAX_DEPTH,
        }
    }
}

impl FollowsGraph {
    /// Build a graph from the declared `inputs = { ... }` block alone.
    ///
    /// Each [`Follows::Indirect`] in `inputs` becomes one
    /// [`EdgeOrigin::Declared`] edge carrying [`Input::range`].
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
    /// Walks `flake.lock` from the root, emitting one [`EdgeOrigin::Resolved`]
    /// edge per `inputs.X = ["a", "b", ...]` follows override, bounded by
    /// [`DEFAULT_MAX_DEPTH`]. Every nested-input path is recorded in
    /// `resolved_universe`, with or without a follows.
    pub fn from_lock(lock: &FlakeLock) -> Self {
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
                        parent_node: nested.path.first().clone(),
                        target_node: nested.path.last().clone(),
                    },
                });
            }
        }
        graph
    }

    /// Build the merged graph: declared edges first, then resolved edges
    /// from the lockfile that no existing edge already covers at the same
    /// `(source, follows)`. Every nested-input path observed in the
    /// lockfile is recorded in `resolved_universe`.
    ///
    /// Dedup is by `(source, follows)`, not by `source` alone: a declared
    /// edge and its resolved sibling can share a source but point at
    /// different targets (the user's depth-N follow points at the user's
    /// top-level input; the lockfile points at the upstream's intermediate
    /// path). Both encode distinct reachability and
    /// [`Self::lock_routes_to`] needs the resolved sibling to survive when
    /// the declared edge is excluded as a candidate.
    pub fn from_flake(inputs: &InputMap, lock: &FlakeLock) -> Self {
        let mut graph = FollowsGraph::from_declared(inputs);
        for nested in lock.nested_inputs() {
            graph.resolved_universe.insert(nested.path.clone());
            if let Some(target) = nested.follows {
                let already = graph
                    .outgoing(&nested.path)
                    .iter()
                    .any(|e| e.follows == target);
                if already {
                    continue;
                }
                graph.insert_edge(Edge {
                    source: nested.path.clone(),
                    follows: target,
                    origin: EdgeOrigin::Resolved {
                        parent_node: nested.path.first().clone(),
                        target_node: nested.path.last().clone(),
                    },
                });
            }
        }
        graph
    }

    /// Override the traversal depth bound. Default is [`DEFAULT_MAX_DEPTH`].
    #[must_use]
    pub fn with_max_depth(mut self, max: usize) -> Self {
        self.max_depth = max;
        self
    }

    /// Outgoing edges from `src`, or an empty slice.
    pub fn outgoing(&self, src: &AttrPath) -> &[Edge] {
        self.edges.get(src).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Iterator over every edge, lex-sorted by source.
    pub fn edges(&self) -> impl Iterator<Item = &Edge> {
        let mut keys: Vec<&AttrPath> = self.edges.keys().collect();
        keys.sort();
        keys.into_iter()
            .flat_map(|k| self.edges.get(k).unwrap().iter())
    }

    /// Edges originating from `flake.nix`. Lex-sorted by source.
    pub fn declared_edges(&self) -> impl Iterator<Item = &Edge> {
        self.edges()
            .filter(|e| matches!(e.origin, EdgeOrigin::Declared { .. }))
    }

    /// Every source path the user has declared a follows for in
    /// `flake.nix`, regardless of whether it resolves to a target. The
    /// union of [`Self::declared_edges`] sources and the nulled
    /// (`follows = ""`) sources tracked alongside them.
    ///
    /// Distinct from [`Self::declared_edges`] because the auto-follow
    /// pipeline must treat a nulled declaration as "already user-owned"
    /// and skip it; the edges-only view drops nulled sources because
    /// they have no target to put on the right-hand side of an edge.
    pub fn declared_sources(&self) -> HashSet<AttrPath> {
        let mut out: HashSet<AttrPath> = self.declared_edges().map(|e| e.source.clone()).collect();
        out.extend(self.declared_nulled_sources.iter().cloned());
        out
    }

    /// Read-only view of source paths declared as `follows = ""`. The
    /// auto-deduplicator consults this to distinguish a nulled
    /// declaration (user explicitly opted out) from a path with a real
    /// follows target.
    pub fn declared_nulled(&self) -> &HashSet<AttrPath> {
        &self.declared_nulled_sources
    }

    /// Cycles among declared edges only: detects per-segment self-cycles
    /// where an edge's source equals its follows. Multi-hop and lockfile-only
    /// cycle detection lives on [`Self::would_create_cycle`].
    pub fn cycles(&self) -> Vec<Cycle> {
        let mut found: Vec<Cycle> = Vec::new();
        let mut seen_keys: HashSet<(AttrPath, AttrPath)> = HashSet::new();
        let mut declared: Vec<&Edge> = self.declared_edges().collect();
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

    /// Declared edges whose source path no longer appears in the lockfile.
    /// A follows declaration whose nested input is gone from `flake.lock`
    /// should be dropped on the next auto-follow pass. Lex-sorted by source.
    pub fn stale_edges(&self) -> Vec<&Edge> {
        let mut declared: Vec<&Edge> = self
            .declared_edges()
            .filter(|e| !self.resolved_universe.contains(&e.source))
            .collect();
        declared.sort_by(|a, b| a.source.cmp(&b.source));
        declared
    }

    /// Sibling of [`Self::stale_edges`] for [`Self::declared_nulled`]:
    /// nulled (`follows = ""`) declarations whose source is absent from
    /// `resolved_universe`. A nulled declaration backed by an
    /// `inputs.X = []` lock entry stays in `resolved_universe` and is not
    /// reported. Lex-sorted.
    pub fn stale_nulled_sources(&self) -> Vec<&AttrPath> {
        let mut sources: Vec<&AttrPath> = self
            .declared_nulled_sources
            .iter()
            .filter(|p| !self.resolved_universe.contains(*p))
            .collect();
        sources.sort();
        sources
    }

    /// Declared follows whose target disagrees with the lockfile's
    /// resolution for the same source path.
    ///
    /// A returned entry means `flake.nix` declares a follows for
    /// `entry.declared.source` pointing at `entry.declared.follows`, but the
    /// lock has either:
    ///
    /// - `lock_target = Some(other)`: a different resolved target, or
    /// - `lock_target = None`: no follows at all (override never applied).
    ///
    /// Both cases call for `nix flake lock`. Sources missing from the
    /// lockfile entirely are reported by [`Self::stale_edges`] instead.
    ///
    /// Lex-sorted by source.
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
        for edge in self.declared_edges() {
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

    /// Whether adding `proposed` would close a follows cycle.
    ///
    /// Origin-agnostic DFS from `proposed.follows`. Reaching `proposed.source`
    /// means the new edge closes a cycle. Beyond the trivial self-edge case,
    /// three classes are covered:
    ///
    /// 1. **Dot-named ancestor.** [`AttrPath`] equality is structural, so a
    ///    participant like `"hls-1.10"` compares by its unquoted segment
    ///    value, not by URL prefix.
    /// 2. **Multi-hop chains.** A cycle `A → B → C → ... → A` is found by
    ///    walking the chain.
    /// 3. **Lockfile-only cycles.** [`EdgeOrigin::Resolved`] edges are
    ///    traversed alongside declared ones, so a chain closing only
    ///    through the lockfile is still reported.
    ///
    /// Bounded by [`Self::with_max_depth`] for malformed graphs. Standard
    /// visited / on-stack sets keep pre-existing cycles from wedging the walk.
    pub fn would_create_cycle(&self, proposed: &Edge) -> bool {
        if is_one_step_cycle(proposed) {
            return true;
        }
        // Structural ancestor case: if the target's leading segment matches
        // any ancestor segment of the source, the edge would point a nested
        // input back at one of its own ancestors -- a self-reference Nix
        // forbids. Catches the dot-named ancestor case even on an empty graph.
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
            // Pre-existing cycle that doesn't pass through `target`. Skip
            // without claiming the proposed edge closes a new one.
            return false;
        }
        // Cycle through structural ancestry: a target inside the current
        // node's subtree, e.g. proposed `c.a -> a` while traversing from `a`
        // whose subtree contains declared edges sourced at `a.x` reaching
        // back to `c.a`.
        if node.is_prefix_of(target) {
            return true;
        }
        on_stack.insert(node.clone());
        // Expand literal-source edges plus edges whose source starts with
        // `node`. The second captures the implicit "parent depends on target"
        // relation a declared `parent.child -> target` follows expresses.
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

    /// Edges describing outgoing dependencies of `node`: those with source
    /// equal to `node`, plus those whose source has `node` as a strict
    /// prefix. The latter encode the implicit "parent depends on target"
    /// relation in a declared `parent.child.follows = target`.
    fn expanded_outgoing(&self, node: &AttrPath) -> Vec<&Edge> {
        let mut out: Vec<&Edge> = Vec::new();
        for (source, edges) in &self.edges {
            if source == node || node.is_prefix_of(source) {
                out.extend(edges.iter());
            }
        }
        out
    }

    /// Group nested inputs by canonical name, in deterministic order.
    ///
    /// `top` is the set of top-level input names already in `flake.nix`.
    /// Sources whose first segment is in `top` are skipped because no
    /// promotion is possible.
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

    /// Drop every edge whose `source` is in `sources`.
    ///
    /// Hides a known set of edges from [`Self::would_create_cycle`] and
    /// [`Self::lock_routes_to`] without re-deriving the graph from
    /// scratch. `resolved_universe` is intentionally untouched: removing
    /// a declared edge does not retroactively unobserve the lockfile
    /// path the source was discovered from.
    pub fn drop_edges_with_sources(&mut self, sources: &[AttrPath]) {
        for src in sources {
            self.edges.remove(src);
        }
    }

    /// Whether the graph routes `source` transitively to `target`, ignoring
    /// `exclude` if given. `extra_edges` are treated as additional
    /// declared-style edges for callers staging follows not yet in the graph.
    ///
    /// Walks both [`EdgeOrigin::Declared`] and [`EdgeOrigin::Resolved`] edges:
    /// [`Self::from_flake`] keeps only one when both encode the same
    /// `(source, target)`, so restricting the walk to one variant would miss
    /// chains closing through the deduped edge.
    pub fn lock_routes_to(
        &self,
        source: &AttrPath,
        target: &AttrPath,
        exclude: Option<&Edge>,
        extra_edges: &[(AttrPath, AttrPath)],
    ) -> bool {
        let mut visited: HashSet<AttrPath> = HashSet::new();
        let mut on_stack: HashSet<AttrPath> = HashSet::new();
        self.dfs_routes_to(
            source,
            target,
            exclude,
            extra_edges,
            0,
            &mut visited,
            &mut on_stack,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn dfs_routes_to(
        &self,
        node: &AttrPath,
        target: &AttrPath,
        exclude: Option<&Edge>,
        extra_edges: &[(AttrPath, AttrPath)],
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
        if visited.contains(node) || on_stack.contains(node) {
            return false;
        }
        if node.is_prefix_of(target) {
            return true;
        }
        on_stack.insert(node.clone());
        let direct_hops = self.expanded_outgoing(node).into_iter().filter_map(|edge| {
            if let Some(skip) = exclude
                && edge.source == skip.source
                && edge.follows == skip.follows
            {
                return None;
            }
            Some(edge.follows.clone())
        });
        for next in direct_hops.collect::<Vec<_>>() {
            if self.dfs_routes_to(
                &next,
                target,
                exclude,
                extra_edges,
                depth + 1,
                visited,
                on_stack,
            ) {
                on_stack.remove(node);
                visited.insert(node.clone());
                return true;
            }
        }
        for (src, dst) in extra_edges {
            if src != node && !node.is_prefix_of(src) {
                continue;
            }
            if let Some(skip) = exclude
                && src == &skip.source
                && dst == &skip.follows
            {
                continue;
            }
            if self.dfs_routes_to(
                dst,
                target,
                exclude,
                extra_edges,
                depth + 1,
                visited,
                on_stack,
            ) {
                on_stack.remove(node);
                visited.insert(node.clone());
                return true;
            }
        }
        // Ancestor rewriting: when `A.B` follows `C`, `A.B.X` resolves to
        // `C.X`. Splice the suffix-after-ancestor onto the target and recurse.
        let mut ancestor: Option<AttrPath> = node.parent();
        while let Some(anc) = ancestor.clone() {
            for edge in self.outgoing(&anc) {
                if let Some(skip) = exclude
                    && edge.source == skip.source
                    && edge.follows == skip.follows
                {
                    continue;
                }
                let mut next = edge.follows.clone();
                for seg in &node.segments()[anc.len()..] {
                    next.push(seg.clone());
                }
                if self.dfs_routes_to(
                    &next,
                    target,
                    exclude,
                    extra_edges,
                    depth + 1,
                    visited,
                    on_stack,
                ) {
                    on_stack.remove(node);
                    visited.insert(node.clone());
                    return true;
                }
            }
            for (src, dst) in extra_edges {
                if src != &anc {
                    continue;
                }
                if let Some(skip) = exclude
                    && src == &skip.source
                    && dst == &skip.follows
                {
                    continue;
                }
                let mut next = dst.clone();
                for seg in &node.segments()[anc.len()..] {
                    next.push(seg.clone());
                }
                if self.dfs_routes_to(
                    &next,
                    target,
                    exclude,
                    extra_edges,
                    depth + 1,
                    visited,
                    on_stack,
                ) {
                    on_stack.remove(node);
                    visited.insert(node.clone());
                    return true;
                }
            }
            ancestor = anc.parent();
        }
        on_stack.remove(node);
        visited.insert(node.clone());
        false
    }
}

fn collect_declared_edges(input: &Input, graph: &mut FollowsGraph) {
    for follows in input.follows() {
        if let Follows::Indirect { path, target } = follows {
            let mut source = AttrPath::new(input.id().clone());
            for seg in path.segments() {
                source.push(seg.clone());
            }
            match target {
                Some(target) => {
                    graph.insert_edge(Edge {
                        source,
                        follows: target.clone(),
                        origin: EdgeOrigin::Declared {
                            range: input.range.clone(),
                        },
                    });
                }
                // `follows = ""`: no edge to insert, but record the
                // source so [`FollowsGraph::declared_sources`] reports
                // it as user-owned.
                None => {
                    graph.declared_nulled_sources.insert(source);
                }
            }
        }
    }
}

/// Self-cycle: source equals follows, structurally.
fn is_one_step_cycle(edge: &Edge) -> bool {
    edge.source == edge.follows
}

/// Whether `url` is a follows reference of the form `"<parent>/<rest>"`.
/// Exposed for consumers that have only a URL string and no typed target.
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
        let g = FollowsGraph::from_lock(&lock);
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
        // `flake.nix` declares `home-manager.nixpkgs.follows`, but the
        // lockfile has no nested input at that path.
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
        let g = FollowsGraph::from_flake(&inputs, &lock);
        let stale: Vec<&Edge> = g.stale_edges();
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].source.to_string(), "home-manager.nixpkgs");
    }

    #[test]
    fn stale_nulled_sources_flags_nulled_without_resolved() {
        let mut input = Input::new(seg("home-manager"));
        input.follows.push(Follows::Indirect {
            path: AttrPath::new(seg("nixpkgs")),
            target: None,
        });
        input.range = Range { start: 1, end: 2 };
        let inputs = make_inputs(vec![input]);

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
        let g = FollowsGraph::from_flake(&inputs, &lock);
        let stale: Vec<String> = g
            .stale_nulled_sources()
            .iter()
            .map(|p| p.to_string())
            .collect();
        assert_eq!(stale, vec!["home-manager.nixpkgs"]);
    }

    /// `inputs.X = []` keeps `X` in `resolved_universe`, so a matching
    /// `follows = ""` is in sync, not stale.
    #[test]
    fn stale_nulled_sources_quiet_when_lock_has_empty_indirect() {
        let mut input = Input::new(seg("home-manager"));
        input.follows.push(Follows::Indirect {
            path: AttrPath::new(seg("nixpkgs")),
            target: None,
        });
        input.range = Range { start: 1, end: 2 };
        let inputs = make_inputs(vec![input]);

        let lock_text = r#"{
  "nodes": {
    "home-manager": {
      "inputs": { "nixpkgs": [] },
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "ddd", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "root": {
      "inputs": { "home-manager": "home-manager" }
    }
  },
  "root": "root",
  "version": 7
}"#;
        let lock = FlakeLock::read_from_str(lock_text).unwrap();
        let g = FollowsGraph::from_flake(&inputs, &lock);
        assert!(g.stale_nulled_sources().is_empty());
    }

    #[test]
    fn self_named_three_segment_declared_round_trips_with_lock() {
        // Regression: a `parent.parent.leaf` lock path must round-trip through
        // the declared edge so [`FollowsGraph::stale_edges`] does not flag it.
        // Hand-build the declared shape the parser produces for
        // `inputs.agenix.inputs.systems.follows = "systems"` inside an
        // `agenix = { ... }` block: `Follows::Indirect` whose `path` carries
        // the full chain `[agenix, systems]`, attached to the owner input
        // `agenix`. Reconstructed source is `agenix.agenix.systems` (3-seg).
        let mut input = Input::new(seg("agenix"));
        input.follows.push(Follows::Indirect {
            path: AttrPath::parse("agenix.systems").unwrap(),
            target: Some(path("systems")),
        });
        input.range = Range { start: 1, end: 2 };
        let inputs = make_inputs(vec![input, declared_input("systems", &[])]);

        let lock_text = r#"{
  "nodes": {
    "agenix": {
      "inputs": { "agenix": "agenix_2" },
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "aaa", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "agenix_2": {
      "inputs": { "systems": "systems_2" },
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "bbb", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "systems": {
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "ccc", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "systems_2": {
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "ddd", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "root": {
      "inputs": { "agenix": "agenix", "systems": "systems" }
    }
  },
  "root": "root",
  "version": 7
}"#;
        let lock = FlakeLock::read_from_str(lock_text).unwrap();
        let g = FollowsGraph::from_flake(&inputs, &lock);

        let declared: Vec<&Edge> = g.declared_edges().collect();
        assert_eq!(declared.len(), 1);
        assert_eq!(declared[0].source.to_string(), "agenix.agenix.systems");

        let stale: Vec<&Edge> = g.stale_edges();
        assert!(
            stale.is_empty(),
            "self-named 3-seg declared edge must not be flagged stale, got: {:?}",
            stale
                .iter()
                .map(|e| e.source.to_string())
                .collect::<Vec<_>>()
        );
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
        // Structural equality on the dot-named segment must catch the
        // ancestor cycle: source `"hls-1.10".nixpkgs`, target `"hls-1.10"`.
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

    /// Multi-hop cycle through declared edges: `A → B`, `B → C`, propose
    /// `C → A`. DFS must traverse the chain rather than rely on a
    /// per-ancestor check on `proposed`.
    #[test]
    fn would_create_cycle_multi_hop_declared() {
        let mut g = FollowsGraph::default();
        g.insert_edge(declared_edge("a", "b"));
        g.insert_edge(declared_edge("b", "c"));
        let proposed = declared_edge("c", "a");
        assert!(g.would_create_cycle(&proposed));
    }

    /// Multi-hop with a `"hls-1.10"` participant: typed [`AttrPath`]
    /// equality must survive the embedded dot.
    #[test]
    fn would_create_cycle_multi_hop_dot_named() {
        let mut g = FollowsGraph::default();
        g.insert_edge(declared_edge("\"hls-1.10\"", "b"));
        g.insert_edge(declared_edge("b", "c"));
        let proposed = declared_edge("c", "\"hls-1.10\"");
        assert!(g.would_create_cycle(&proposed));
    }

    /// Lockfile-only cycle: a 2-hop chain closing only through resolved
    /// edges. Origin-agnostic DFS must report it.
    #[test]
    fn would_create_cycle_lockfile_only() {
        let mut g = FollowsGraph::default();
        g.insert_edge(Edge {
            source: AttrPath::parse("treefmt-nix.nixpkgs").unwrap(),
            follows: AttrPath::parse("harmonia.treefmt-nix").unwrap(),
            origin: EdgeOrigin::Resolved {
                parent_node: seg("treefmt-nix"),
                target_node: seg("harmonia"),
            },
        });
        g.insert_edge(Edge {
            source: AttrPath::parse("harmonia.treefmt-nix").unwrap(),
            follows: AttrPath::parse("treefmt-nix").unwrap(),
            origin: EdgeOrigin::Resolved {
                parent_node: seg("harmonia"),
                target_node: seg("treefmt-nix"),
            },
        });
        let proposed = Edge {
            source: AttrPath::parse("treefmt-nix").unwrap(),
            follows: AttrPath::parse("treefmt-nix.nixpkgs").unwrap(),
            origin: EdgeOrigin::Declared {
                range: Range { start: 0, end: 0 },
            },
        };
        assert!(g.would_create_cycle(&proposed));
    }

    /// DFS terminates and still reports a cycle-closing proposal when the
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

    /// `max_depth` bounds DFS so a malformed graph cannot wedge it.
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
    fn drop_edges_with_sources_removes_only_listed_sources() {
        let mut g = FollowsGraph::default();
        g.insert_edge(declared_edge("crane.nixpkgs", "nixpkgs"));
        g.insert_edge(declared_edge("crane.flake-utils", "flake-utils"));
        g.insert_edge(declared_edge("treefmt-nix.nixpkgs", "nixpkgs"));

        g.drop_edges_with_sources(&[path("crane.nixpkgs")]);

        let remaining: Vec<String> = g.edges().map(|e| e.source.to_string()).collect();
        assert_eq!(
            remaining,
            vec![
                "crane.flake-utils".to_string(),
                "treefmt-nix.nixpkgs".to_string(),
            ],
            "only the listed source should be dropped; the rest survive intact"
        );
    }

    #[test]
    fn drop_edges_with_sources_clears_cycle_through_dropped_edge() {
        // Edges `X.Y -> Y` and `Y.Z -> Z` chain via [`Self::would_create_cycle`]
        // to reject a `Z.X -> X` candidate (the `Z.is_prefix_of(Z.X)`
        // shortcut fires at the visited `Z` node). Dropping `X.Y` must
        // clear the path.
        let mut g = FollowsGraph::default();
        g.insert_edge(declared_edge("X.Y", "Y"));
        g.insert_edge(declared_edge("Y.Z", "Z"));
        let proposed = declared_edge("Z.X", "X");
        assert!(g.would_create_cycle(&proposed));

        g.drop_edges_with_sources(&[path("X.Y")]);
        assert!(!g.would_create_cycle(&proposed));
    }

    #[test]
    fn cycles_finds_self_referential_declared_edge() {
        let mut inputs = InputMap::new();
        let mut input = Input::new(seg("foo"));
        input.follows.push(Follows::Indirect {
            path: AttrPath::new(seg("foo")),
            target: Some(AttrPath::parse("foo.foo").unwrap()),
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
        let g = FollowsGraph::from_flake(&inputs, &lock);
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
        let g = FollowsGraph::from_flake(&inputs, &lock);
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
        let g = FollowsGraph::from_flake(&inputs, &lock);
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
        let g = FollowsGraph::from_flake(&inputs, &lock);
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
        let g = FollowsGraph::from_flake(&inputs, &lock);
        let edges: Vec<&Edge> = g.outgoing(&path("treefmt-nix.nixpkgs")).iter().collect();
        assert_eq!(edges.len(), 1);
        assert!(matches!(edges[0].origin, EdgeOrigin::Declared { .. }));
    }

    #[test]
    fn lock_routes_to_chain_through_user_depth1() {
        let mut g = FollowsGraph::default();
        g.insert_edge(Edge {
            source: path("hyprland.aquamarine.nixpkgs"),
            follows: path("hyprland.nixpkgs"),
            origin: EdgeOrigin::Resolved {
                parent_node: seg("aquamarine"),
                target_node: seg("nixpkgs"),
            },
        });
        g.insert_edge(declared_edge("hyprland.nixpkgs", "nixpkgs"));
        assert!(g.lock_routes_to(
            &path("hyprland.aquamarine.nixpkgs"),
            &path("nixpkgs"),
            None,
            &[],
        ));
    }

    #[test]
    fn lock_routes_to_excludes_candidate_edge() {
        let mut g = FollowsGraph::default();
        g.insert_edge(Edge {
            source: path("hyprland.aquamarine.nixpkgs"),
            follows: path("hyprland.nixpkgs"),
            origin: EdgeOrigin::Resolved {
                parent_node: seg("aquamarine"),
                target_node: seg("nixpkgs"),
            },
        });
        g.insert_edge(declared_edge("hyprland.nixpkgs", "nixpkgs"));
        let candidate = declared_edge("hyprland.aquamarine.nixpkgs", "nixpkgs");
        g.insert_edge(candidate.clone());
        assert!(g.lock_routes_to(&candidate.source, &candidate.follows, Some(&candidate), &[],));
    }

    /// Auto-remove must not drop a load-bearing follow.
    #[test]
    fn lock_routes_to_no_other_path_returns_false_when_candidate_excluded() {
        let mut g = FollowsGraph::default();
        let candidate = declared_edge("a.b", "nixpkgs");
        g.insert_edge(candidate.clone());
        assert!(!g.lock_routes_to(&candidate.source, &candidate.follows, Some(&candidate), &[]));
    }

    #[test]
    fn lock_routes_to_no_route_returns_false() {
        let g = FollowsGraph::default();
        assert!(!g.lock_routes_to(&path("a.b.c"), &path("nixpkgs"), None, &[]));
    }

    /// Deep upstream chain `a.b.c.d -> a.b.c -> a.b -> a`, all Resolved.
    #[test]
    fn lock_routes_to_deep_upstream_chain() {
        let mut g = FollowsGraph::default();
        for (src, dst) in [("a.b.c.d", "a.b.c"), ("a.b.c", "a.b"), ("a.b", "a")] {
            g.insert_edge(Edge {
                source: path(src),
                follows: path(dst),
                origin: EdgeOrigin::Resolved {
                    parent_node: Segment::from_unquoted(src).unwrap(),
                    target_node: Segment::from_unquoted(dst).unwrap(),
                },
            });
        }
        assert!(g.lock_routes_to(&path("a.b.c.d"), &path("a"), None, &[]));
    }

    /// End-to-end: a depth-3 chain detected by [`FollowsGraph::lock_routes_to`]
    /// drives a [`Change::Remove`] through [`FlakeEdit`].
    #[test]
    fn lock_routes_to_drives_change_remove_at_depth_three() {
        use crate::change::{Change, ChangeId};
        use crate::edit::FlakeEdit;

        let mut g = FollowsGraph::default();
        for (src, dst) in [
            ("omnibus.flops.POP.nixpkgs", "omnibus.flops.nixpkgs"),
            ("omnibus.flops.nixpkgs", "omnibus.nixpkgs"),
            ("omnibus.nixpkgs", "nixpkgs"),
        ] {
            g.insert_edge(Edge {
                source: path(src),
                follows: path(dst),
                origin: EdgeOrigin::Resolved {
                    parent_node: Segment::from_unquoted(src).unwrap(),
                    target_node: Segment::from_unquoted(dst).unwrap(),
                },
            });
        }
        let candidate = declared_edge("omnibus.flops.POP.nixpkgs", "nixpkgs");
        g.insert_edge(candidate.clone());

        assert!(
            g.lock_routes_to(&candidate.source, &candidate.follows, Some(&candidate), &[]),
            "depth-3 chain should be predicted as covered by upstream propagation"
        );

        let flake = r#"{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    omnibus.url = "github:Lehmanator/nix-configs";
    omnibus.inputs.nixpkgs.follows = "nixpkgs";
    omnibus.inputs.flops.inputs.POP.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, ... }: { };
}
"#;
        let mut fe = FlakeEdit::from_text(flake).expect("parses");
        let change = Change::Remove {
            ids: vec![ChangeId::new(candidate.source.clone())],
        };
        let outcome = fe.apply_change(change).expect("apply succeeds");
        let new_text = outcome.text.expect("walker rewrote the tree");

        assert!(
            !new_text.contains("flops"),
            "depth-3 redundant follows should be removed, got:\n{new_text}"
        );
        assert!(
            new_text.contains("omnibus.inputs.nixpkgs.follows = \"nixpkgs\""),
            "load-bearing depth-1 follows must remain, got:\n{new_text}"
        );
    }
}
