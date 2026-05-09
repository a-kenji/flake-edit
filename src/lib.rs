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
//! - [`validate`] runs pre-edit lint passes. [`Error`] is the crate-wide
//!   error.
//!
//! Feature flags: `application` (default) enables the binary-side glue
//! ([`app`], [`cli`], [`diff`], [`tui`]) and pulls in `clap`, `ratatui`,
//! `crossterm`, `diffy`, etc. Library-only consumers can disable it with
//! `--no-default-features` to compile the pure edit / walk / forge surface.

#[cfg(feature = "application")]
pub mod app;
pub mod cache;
pub mod change;
#[cfg(feature = "application")]
pub mod cli;
pub mod config;
#[cfg(feature = "application")]
pub mod diff;
pub mod edit;
pub mod error;
pub mod follows;
pub mod forge;
pub mod input;
pub mod lock;
#[cfg(feature = "application")]
pub mod tui;
pub mod uri;
pub mod validate;
pub mod walk;

pub use error::Error;
