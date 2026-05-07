//! Edit Nix flake inputs from Rust.
//!
//! [`edit::FlakeEdit`] is the entry point: build one with
//! [`edit::FlakeEdit::from_text`], queue or apply [`change::Change`]s, and read
//! back the new source via [`edit::FlakeEdit::source_text`] or
//! [`edit::ApplyOutcome`].
//!
//! Supporting modules:
//! - [`walk`] traverses the `rnix` CST and applies edits.
//! - [`input`] models a flake input and its [`input::Follows`] declarations.
//! - [`lock`] parses `flake.lock` for revs, follows targets, and nested input
//!   discovery via [`lock::FlakeLock::nested_inputs`].
//! - [`forge`] talks to GitHub / Gitea / Forgejo, normalizes versions, and
//!   applies pin/unpin updates (`forge::api`, `forge::channel`,
//!   `forge::version`, `forge::update`).
//! - [`config`] loads `flake-edit.toml`.
//! - [`cache`] persists URI completion state.
//! - [`validate`] runs pre-edit lint passes. [`error::FlakeEditError`] is the
//!   crate-wide error.
//!
//! Feature flags: `tui` enables [`app`] and [`tui`], `diff` enables [`diff`].

#[cfg(feature = "tui")]
pub mod app;
pub mod cache;
pub mod change;
pub mod cli;
pub mod config;
#[cfg(feature = "diff")]
pub mod diff;
pub mod edit;
pub mod error;
pub mod follows;
pub mod forge;
pub mod input;
pub mod lock;
#[cfg(feature = "tui")]
pub mod tui;
pub mod uri;
pub mod validate;
pub mod walk;
