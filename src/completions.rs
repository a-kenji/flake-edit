//! Dynamic shell completions for clap's CompleteEnv system.
//!
//! These functions provide runtime completions for Bash, Zsh, and Fish shells.
//! They are called by clap when the user presses TAB during command entry.

use std::ffi::OsStr;

use clap_complete::engine::CompletionCandidate;

use crate::cache::{Cache, DEFAULT_URI_TYPES};
use crate::edit::FlakeEdit;
use crate::lock::FlakeLock;

/// Try to load FlakeEdit from flake.nix in the current directory.
fn load_flake_edit() -> Option<FlakeEdit> {
    let content = std::fs::read_to_string("flake.nix").ok()?;
    FlakeEdit::from_text(&content).ok()
}

/// Complete input IDs for commands that operate on all inputs (remove, change).
///
/// Returns all input IDs including nested follows relationships like "rust-overlay.nixpkgs".
pub fn complete_inputs(current: &OsStr) -> Vec<CompletionCandidate> {
    let current = current.to_string_lossy();
    let mut candidates = Vec::new();

    // Try to load flake.nix from current directory
    if let Some(mut flake_edit) = load_flake_edit() {
        let inputs = flake_edit.list();
        for (id, input) in inputs {
            // Add the top-level input
            if id.starts_with(current.as_ref()) {
                candidates.push(CompletionCandidate::new(id.clone()));
            }

            // Add any follows relationships as completable paths
            for follows in input.follows() {
                if let crate::input::Follows::Indirect(from, _) = follows {
                    let path = format!("{}.{}", id, from);
                    if path.starts_with(current.as_ref()) {
                        candidates.push(CompletionCandidate::new(path));
                    }
                }
            }
        }
    }

    candidates
}

/// Complete top-level input IDs only (pin, unpin, update).
///
/// Returns only the top-level input names, not nested paths.
pub fn complete_toplevel_inputs(current: &OsStr) -> Vec<CompletionCandidate> {
    let current = current.to_string_lossy();
    let mut candidates = Vec::new();

    if let Some(mut flake_edit) = load_flake_edit() {
        let inputs = flake_edit.list();
        for id in inputs.keys() {
            if id.starts_with(current.as_ref()) {
                candidates.push(CompletionCandidate::new(id.clone()));
            }
        }
    }

    candidates
}

/// Complete URIs for the 'add' command.
///
/// Returns URI type prefixes (github:, gitlab:, etc.) and cached URIs.
pub fn complete_uris(current: &OsStr) -> Vec<CompletionCandidate> {
    let current = current.to_string_lossy();
    let mut candidates = Vec::new();

    // Add URI type prefixes
    for uri_type in DEFAULT_URI_TYPES {
        if uri_type.starts_with(current.as_ref()) {
            candidates.push(CompletionCandidate::new(uri_type));
        }
    }

    // Add cached URIs
    let cache = Cache::load();
    for uri in cache.list_uris() {
        if uri.starts_with(current.as_ref()) {
            candidates.push(CompletionCandidate::new(uri));
        }
    }

    candidates
}

/// Complete nested input paths for the 'follow' command's input argument.
///
/// Returns paths like "rust-overlay.nixpkgs" that can be followed.
pub fn complete_follow_paths(current: &OsStr) -> Vec<CompletionCandidate> {
    let current = current.to_string_lossy();
    let mut candidates = Vec::new();

    if let Ok(lock) = FlakeLock::from_default_path() {
        for path in lock.nested_input_paths() {
            if path.starts_with(current.as_ref()) {
                candidates.push(CompletionCandidate::new(path));
            }
        }
    }

    candidates
}
