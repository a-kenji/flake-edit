//! Test helpers for [`flake_edit::follows::FollowsGraph`].
//!
//! Shared assertion primitives for integration tests that build graphs
//! from on-disk fixtures. Every helper accepts attribute paths as `&str`
//! and parses them through [`AttrPath::parse`].

mod common;

use common::load_fixtures;
use flake_edit::edit::FlakeEdit;
use flake_edit::follows::{AttrPath, Edge, FollowsGraph};
use flake_edit::lock::FlakeLock;

/// Build the [`FollowsGraph`] for a fixture pair (`<name>.flake.nix` +
/// `<name>.flake.lock`).
pub fn build_graph(fixture: &str) -> FollowsGraph {
    let (flake_nix, flake_lock) = load_fixtures(fixture);
    let mut edit = FlakeEdit::from_text(&flake_nix).expect("fixture flake.nix parses");
    let inputs = edit.list().clone();
    let lock = FlakeLock::read_from_str(&flake_lock).expect("fixture flake.lock parses");
    FollowsGraph::from_flake(&inputs, &lock)
}

#[track_caller]
pub fn assert_graph_has_edge(g: &FollowsGraph, parent: &str, nested: &str, target: &str) {
    let parent_path = AttrPath::parse(parent).expect("parent parses as AttrPath");
    let nested_seg =
        flake_edit::follows::Segment::from_unquoted(nested).expect("nested segment is unquoted");
    let mut source = parent_path;
    source.push(nested_seg);
    let target_path = AttrPath::parse(target).expect("target parses as AttrPath");
    let outgoing = g.outgoing(&source);
    assert!(
        outgoing.iter().any(|e| e.follows == target_path),
        "expected edge {source} -> {target_path}; outgoing: {outgoing:?}",
    );
}

#[track_caller]
pub fn assert_no_cycle(g: &FollowsGraph) {
    let cycles = g.cycles();
    assert!(
        cycles.is_empty(),
        "expected no cycles in graph, found: {cycles:?}",
    );
}

#[track_caller]
pub fn assert_cycle_contains(g: &FollowsGraph, attr_path: &str) {
    let needle = AttrPath::parse(attr_path).expect("cycle attr_path parses");
    let cycles = g.cycles();
    let hit = cycles.iter().any(|c| {
        c.edges
            .iter()
            .any(|e| e.source == needle || e.follows == needle)
    });
    assert!(
        hit,
        "expected a cycle containing {needle}; cycles: {cycles:?}",
    );
}

#[track_caller]
pub fn assert_stale_edges(g: &FollowsGraph, expected: &[&str]) {
    let mut got: Vec<String> = g
        .stale_edges()
        .iter()
        .map(|e| e.source.to_string())
        .collect();
    got.sort();
    let mut want: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
    want.sort();
    assert_eq!(
        got, want,
        "stale_edges() mismatch: got {got:?}, want {want:?}",
    );
}

/// Convenience: assert that adding `proposed_source` -> `proposed_target` to
/// the graph would close a cycle. Source / target parsed via [`AttrPath::parse`].
#[track_caller]
pub fn assert_would_create_cycle(g: &FollowsGraph, source: &str, follows: &str) {
    let edge = make_proposed_edge(source, follows);
    assert!(
        g.would_create_cycle(&edge),
        "expected proposed edge {source} -> {follows} to close a cycle",
    );
}

#[track_caller]
pub fn assert_would_not_create_cycle(g: &FollowsGraph, source: &str, follows: &str) {
    let edge = make_proposed_edge(source, follows);
    assert!(
        !g.would_create_cycle(&edge),
        "expected proposed edge {source} -> {follows} to not close a cycle",
    );
}

fn make_proposed_edge(source: &str, follows: &str) -> Edge {
    use flake_edit::follows::EdgeOrigin;
    use flake_edit::input::Range;
    Edge {
        source: AttrPath::parse(source).expect("source parses as AttrPath"),
        follows: AttrPath::parse(follows).expect("follows parses as AttrPath"),
        origin: EdgeOrigin::Declared {
            range: Range { start: 0, end: 0 },
        },
    }
}

#[test]
fn helpers_build_graph_for_multi_hop_cycle() {
    let g = build_graph("multi_hop_cycle");
    // The flake.nix declares the chain `a.b -> b` and `b.c -> c`.
    // The graph has no cycles in its declared form; the resolver's
    // multi-hop logic is exercised via the inner `would_create_cycle`
    // unit tests in `src/follows/graph.rs`. Here we just pin the edges.
    assert_graph_has_edge(&g, "a", "b", "b");
    assert_graph_has_edge(&g, "b", "c", "c");
    assert_no_cycle(&g);
}

#[test]
fn helpers_build_graph_for_dot_ancestor_cycle() {
    let g = build_graph("dot_ancestor_cycle");
    assert_graph_has_edge(&g, "helper", "nixpkgs", "nixpkgs");
    assert_graph_has_edge(&g, "\"hls-1.10\"", "helper", "helper");
    assert_no_cycle(&g);
}

#[test]
fn helpers_build_graph_for_lockfile_only_cycle() {
    let g = build_graph("lockfile_only_cycle");
    // flake.nix declares no follows; the lockfile alone produces
    // `foo.bar -> bar` and `bar.foo -> foo` as resolved edges.
    assert_no_cycle(&g);
    // The merged graph still contains the resolved edges from the lockfile,
    // even though declared form is empty.
    assert_graph_has_edge(&g, "foo", "bar", "bar");
    assert_graph_has_edge(&g, "bar", "foo", "foo");
}

#[test]
fn helpers_build_graph_for_stale_lockfile_only() {
    let g = build_graph("stale_lockfile_only");
    // Both `foo.bar` and `foo.baz` are declared in flake.nix but neither
    // nested input exists in the lockfile.
    assert_stale_edges(&g, &["foo.bar", "foo.baz"]);
}
