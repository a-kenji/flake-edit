use std::path::PathBuf;

use crate::lock::LockError;
use crate::validate::ValidationError;
use crate::walk::WalkerError;

/// Error for [`crate::edit::FlakeEdit`] operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Failed to read a flake file. Carries the path that the read was
    /// attempted against so the caller can surface it.
    #[error("failed to read {path}", path = path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// Failed to write a flake file.
    #[error("failed to write {path}", path = path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// The CST walker rejected a change. See [`WalkerError`] for details.
    #[error(transparent)]
    Walker(#[from] WalkerError),
    /// A failure parsing or walking `flake.lock`. See [`LockError`] for the
    /// per-variant breakdown.
    #[error(transparent)]
    Lock(#[from] LockError),
    /// Tried to add an input that already exists. The wrapped string is the
    /// existing input id.
    #[error("input '{0}' already exists in the flake")]
    DuplicateInput(String),
    /// Tried to operate on an input id that is not declared in the flake.
    #[error("input '{0}' not found in the flake")]
    InputNotFound(String),
    /// Tried to toggle an input that has no url binding (e.g. a
    /// follows-only input).
    #[error("input '{0}' has no url to toggle (follows-only input)")]
    NoUrlToToggle(String),
    /// Tried to remove an input's active url without a stored alternate to
    /// take its place; honoring it would leave the input url-less.
    #[error("cannot remove the active url of '{0}' without an alternate to activate")]
    RemoveActiveWithoutAlternate(String),
    /// The `add-follow` subcommand received a path deeper than `parent.child`.
    /// `flake-edit follow` accepts deeper paths, bounded by
    /// [`crate::config::FollowConfig::max_depth`] when that is set; this
    /// guard catches typos in the explicit-path command before they produce
    /// nested `inputs.*.inputs.*.follows` chains.
    #[error(
        "`add-follow` accepts only depth-1 paths of the form `parent.child`; got '{path}' ({segments} segments)"
    )]
    AddFollowDepthLimit { path: String, segments: usize },
    /// Pre-edit validation found one or more fatal issues in `flake.nix`.
    #[error("validation failed in flake.nix ({} issue(s))", .0.len())]
    Validation(Vec<ValidationError>),
}

impl Error {
    /// Actionable hint to display alongside the error, when one exists.
    ///
    /// Hints live here rather than in `#[error(...)]` strings so the binary
    /// can render them on a separate `hint:` line and library callers can
    /// choose to surface or ignore them.
    pub fn hint(&self) -> Option<String> {
        match self {
            Self::DuplicateInput(id) => Some(format!(
                "to replace it, run `flake-edit remove {id}` then `flake-edit add {id} <flakeref>`; \
                 or add it under a different id with `flake-edit add [ID] <flakeref>`"
            )),
            Self::InputNotFound(id) => Some(format!(
                "to add it, run `flake-edit add {id} <flakeref>`; \
                 see declared inputs with `flake-edit list`"
            )),
            Self::AddFollowDepthLimit { .. } => Some(
                "use `flake-edit follow` for deeper paths (depth bounded by `follow.max_depth` in your config, if set)"
                    .into(),
            ),
            _ => None,
        }
    }

    /// Per-error rendering of a `Validation` aggregate as one bullet per
    /// inner error. Returns `None` for non-aggregate variants.
    pub fn bullets(&self) -> Option<Vec<String>> {
        match self {
            Self::Validation(errors) => Some(errors.iter().map(|e| e.to_string()).collect()),
            _ => None,
        }
    }
}
