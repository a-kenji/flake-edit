//! Shared test fixtures and utilities for flake-edit tests.
//!
//! This module provides helpers for loading test data and rstest fixtures.

#![allow(dead_code)]

use flake_edit::change::Change;
use flake_edit::edit::FlakeEdit;
use flake_edit::walk::Walker;
use rstest::fixture;

/// Load a fixture file pair (flake.nix and flake.lock).
pub fn load_fixtures(name: &str) -> (String, String) {
    let dir = env!("CARGO_MANIFEST_DIR");
    let flake_nix =
        std::fs::read_to_string(format!("{dir}/tests/fixtures/{name}.flake.nix")).unwrap();
    let flake_lock =
        std::fs::read_to_string(format!("{dir}/tests/fixtures/{name}.flake.lock")).unwrap();
    (flake_nix, flake_lock)
}

/// Load just the flake.nix content.
pub fn load_flake(name: &str) -> String {
    let dir = env!("CARGO_MANIFEST_DIR");
    std::fs::read_to_string(format!("{dir}/tests/fixtures/{name}.flake.nix")).unwrap()
}

/// Info struct for snapshot metadata.
#[derive(serde::Serialize)]
pub struct Info {
    pub flake_nix: String,
    pub changes: Vec<Change>,
}

impl Info {
    pub fn new(flake_nix: String, changes: Vec<Change>) -> Self {
        Self { flake_nix, changes }
    }

    pub fn empty() -> Self {
        Self::new(String::new(), vec![])
    }

    pub fn with_change(change: Change) -> Self {
        Self::new(String::new(), vec![change])
    }
}

/// Available fixture names for testing.
pub const FIXTURES: &[&str] = &[
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
