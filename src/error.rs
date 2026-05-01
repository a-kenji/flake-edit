use thiserror::Error;

use crate::validate::ValidationError;
use crate::walk::WalkerError;

/// Error for [`crate::edit::FlakeEdit`] operations.
#[derive(Debug, Error)]
pub enum FlakeEditError {
    /// An I/O operation on `flake.nix` or `flake.lock` failed.
    #[error("IoError: {0}")]
    Io(#[from] std::io::Error),
    /// The CST walker rejected a change. See [`WalkerError`] for details.
    #[error(transparent)]
    Walker(#[from] WalkerError),
    /// `flake.lock` has no `root` node referenced by `nodes`.
    #[error("Lock file missing root node")]
    LockMissingRoot,
    /// Generic lockfile-shape failure: missing fields, malformed paths,
    /// unresolvable input references.
    #[error("There is an error in the Lockfile: {0}")]
    LockError(String),
    /// JSON deserialization of `flake.lock` failed.
    #[error("Deserialization Error: {0}")]
    Serde(#[from] serde_json::Error),
    /// Tried to add an input that already exists. The wrapped string is the
    /// existing input id.
    #[error(
        "Input '{0}' already exists in the flake.\n\nTo replace it:\n  1. Remove it first: flake-edit remove {0}\n  2. Then add it again: flake-edit add {0} <flakeref>\n\nOr add it with a different [ID]:\n  flake-edit add [ID] <flakeref>\n\nTo see all current inputs: flake-edit list"
    )]
    DuplicateInput(String),
    /// Tried to operate on an input id that is not declared in the flake.
    #[error(
        "Input '{0}' not found in the flake.\n\nTo add it:\n  flake-edit add {0} <flakeref>\n\nTo see all current inputs: flake-edit list"
    )]
    InputNotFound(String),
    /// The `add-follow` subcommand received a path deeper than `parent.child`.
    /// `flake-edit follow` accepts deeper paths up to
    /// [`crate::config::FollowConfig::max_depth`]; this guard catches typos
    /// in the explicit-path command before they produce nested
    /// `inputs.*.inputs.*.follows` chains.
    #[error(
        "`add-follow` accepts only depth-1 paths of the form `parent.child`; got '{path}' ({segments} segments).\n\nUse `flake-edit follow` for deeper paths (depth bounded by `follow.max_depth` in your config)."
    )]
    AddFollowDepthLimit { path: String, segments: usize },
    /// Pre-edit validation found one or more fatal issues in `flake.nix`.
    #[error("Validation error in flake.nix:\n{}", format_validation_errors(.0))]
    Validation(Vec<ValidationError>),
}

fn format_validation_errors(errors: &[ValidationError]) -> String {
    errors
        .iter()
        .map(|e| format!("  - {}", e))
        .collect::<Vec<_>>()
        .join("\n")
}
