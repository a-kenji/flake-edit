//! Shared test fixtures and utilities for flake-edit tests.
//!
//! This module provides helpers for loading test data and rstest fixtures.

#![expect(dead_code)]

use flake_edit::change::Change;
use flake_edit::edit::FlakeEdit;
use flake_edit::walk::Walker;
use rstest::fixture;

/// Load a fixture file pair (flake.nix and flake.lock).
pub(crate) fn load_fixtures(name: &str) -> (String, String) {
    let dir = env!("CARGO_MANIFEST_DIR");
    let flake_nix =
        std::fs::read_to_string(format!("{dir}/tests/fixtures/{name}.flake.nix")).unwrap();
    let flake_lock =
        std::fs::read_to_string(format!("{dir}/tests/fixtures/{name}.flake.lock")).unwrap();
    (flake_nix, flake_lock)
}

/// Load just the flake.nix content.
pub(crate) fn load_flake(name: &str) -> String {
    let dir = env!("CARGO_MANIFEST_DIR");
    std::fs::read_to_string(format!("{dir}/tests/fixtures/{name}.flake.nix")).unwrap()
}

/// Info struct for snapshot metadata.
#[derive(serde::Serialize)]
pub(crate) struct Info {
    pub(crate) flake_nix: String,
    pub(crate) changes: Vec<Change>,
}

impl Info {
    pub(crate) fn new(flake_nix: String, changes: Vec<Change>) -> Self {
        Self { flake_nix, changes }
    }

    pub(crate) fn empty() -> Self {
        Self::new(String::new(), vec![])
    }

    pub(crate) fn with_change(change: Change) -> Self {
        Self::new(String::new(), vec![change])
    }
}

/// Available fixture names for testing.
pub(crate) const FIXTURES: &[&str] = &[
    "root",
    "root_alt",
    "toplevel_nesting",
    "completely_flat_toplevel",
    "completely_flat_toplevel_alt",
    "completely_flat_toplevel_not_a_flake",
    "completely_flat_toplevel_not_a_flake_nested",
    "one_level_nesting_flat",
    "one_level_nesting_flat_not_a_flake",
    "flat_nested_flat",
    "first_nested_node",
    // multi-hop / dot-ancestor / lockfile-only cycle resolver fixtures
    "multi_hop_cycle",
    "dot_ancestor_cycle",
    "lockfile_only_cycle",
    "stale_lockfile_only",
    "split_inputs_block_and_flat",
    "stale_lock",
    // depth-2 follows fixtures
    "transitive_grandchild",
    "transitive_grandchild_existing",
    "transitive_grandchild_cycle",
    "transitive_self_named",
    "transitive_promote_unlocks_deeper",
    // depth-N upstream-redundancy fixtures
    "depth_upstream_redundant",
    "depth_upstream_partial",
    // depth-N upstream-redundancy: auto-removal of already-declared follows
    "depth_upstream_redundant_declared",
    "depth_upstream_redundant_partial",
    "depth_upstream_redundant_depth3",
    // transitive_min=2 promotion alongside upstream-redundant nested follow
    "transitive_promote_with_upstream_redundant",
    // stale-edge removal that simultaneously unblocks a follow candidate
    "stale_edge_unblocks_follow",
    // empty-Indirect (`inputs.X = []`) regression fixture
    "lock_indirect_empty",
    // empty-target follows (`inputs.X.follows = ""`)
    "follows_empty_target",
    // user-declared nulled nested follow that must not be retargeted by
    // the auto-deduplicator
    "follow_respects_nulled",
    // stale-source removal for nulled `follows = ""` declarations
    "nulled_stale_source_removed",
    "nulled_stale_depth3",
    // ancestor-declared nested follow that must not be overridden by a
    // deeper auto-emission with the same trailing segment
    "follow_respects_ancestor",
    // inputless flake (no `inputs = { ... };` block)
    "follow_no_inputs",
    // slash-form follows target (`follows = "a/b"`)
    "follow_slash_syntax",
    // depth-N redundant follow that is the only entry inside an
    // `inputs = { ... }` block; auto-removal must prune the now-empty
    // intermediate block too
    "prune_empty_intermediate_inputs",
    // nested URL override on a transitive input must not be parsed as a
    // follows declaration
    "nested_url_override",
];

/// Create a Walker from a fixture name.
#[fixture]
pub fn walker(#[default("root")] fixture_name: &str) -> Walker {
    let content = load_flake(fixture_name);
    Walker::new(&content)
}

/// Create a FlakeEdit from a fixture name.
#[fixture]
pub fn editor(#[default("root")] fixture_name: &str) -> FlakeEdit {
    let content = load_flake(fixture_name);
    FlakeEdit::from_text(&content).unwrap()
}
