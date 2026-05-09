use std::path::PathBuf;

use crate::change::ChangeId;
use crate::config::ConfigError;
use crate::follows::path::AttrPathParseError;
use crate::validate::ValidationError;

/// Errors raised inside the binary's command and handler layer.
///
/// One layer combining the per-subcommand operations and the top-level
/// dispatcher.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A failure inside the library edit / walk / lock layer.
    #[error(transparent)]
    Flake(#[from] crate::Error),

    /// A configuration loading failure.
    #[error(transparent)]
    Config(#[from] ConfigError),

    /// An io error not otherwise classified (e.g. nix subprocess failure).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// `flake.nix` could not be opened or located.
    #[error("could not open flake.nix at {path}", path = path.display())]
    FlakeNotFound {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// The directory passed to `--flake` exists but contains no `flake.nix`.
    #[error("no flake.nix in directory {path}", path = path.display())]
    FlakeDirEmpty { path: PathBuf },

    /// `--flake` and `--lock` were combined with the batch
    /// `follow [PATHS...]` form, which owns its own per-file editor.
    #[error("`--flake` and `--lock` cannot be used with `follow [PATHS]`")]
    IncompatibleFollowOptions,

    /// A subcommand was invoked without a URI argument when one is required.
    #[error("no URI provided")]
    NoUri,

    /// A subcommand was invoked without an input id when one is required.
    #[error("no input id provided")]
    NoId,

    /// An input list was empty when at least one was required.
    #[error("no inputs found in the flake")]
    NoInputs,

    /// A flake reference could not be parsed by `nix_uri`.
    #[error("invalid URI '{uri}'")]
    InvalidUri {
        uri: String,
        #[source]
        source: nix_uri::NixUriError,
    },

    /// An input id was malformed; carries the typed parse error.
    #[error("invalid input id '{id}'")]
    InvalidInputId {
        id: String,
        #[source]
        source: AttrPathParseError,
    },

    /// A follows path was malformed; carries the typed parse error.
    #[error("invalid follows path '{path}'")]
    InvalidFollowsPath {
        path: String,
        #[source]
        source: AttrPathParseError,
    },

    /// `nix_uri` rendered a flake reference but could not infer an id from it.
    #[error("could not infer id from flake reference '{uri}'")]
    CouldNotInferId { uri: String },

    /// `nix_uri` failed to apply uri options to a parsed flake reference.
    #[error("could not apply uri options to '{uri}'")]
    ApplyUriOptions {
        uri: String,
        #[source]
        source: nix_uri::NixUriError,
    },

    /// The named input has no concrete URL to pin against (e.g. a
    /// `follows`-only input or a non-standard reference shape).
    #[error("input '{id}' has no pinnable URL (it may use follows or a non-standard format)")]
    InputNotPinnable { id: String },

    /// Removing an input did not produce a syntax change.
    #[error("could not remove input '{id}'")]
    CouldNotRemove { id: ChangeId },

    /// Could not load `flake.lock`. The wrapped library error already
    /// classifies the underlying failure.
    #[error("could not read lock file '{path}'", path = path.display())]
    LockFile {
        path: PathBuf,
        #[source]
        source: crate::Error,
    },

    /// A `follow <input> <target>` invocation could not establish the
    /// follows relationship.
    #[error("could not create follows relationship for '{id}'")]
    FollowsCreateFailed { id: String },

    /// Validation of `flake.nix` failed after applying speculative edits.
    /// Distinct from `crate::Error::Validation` (which fires before edits)
    /// because the diagnostic flow needs to render the staged edits too.
    #[error("validation failed after applying edits ({} issue(s))", .0.len())]
    ValidationAfterEdit(Vec<ValidationError>),

    /// Aggregated failures from a `follow [PATHS...]` batch. Each entry
    /// pairs the offending path with the error processing it produced.
    #[error("{} file(s) failed during batch processing", failures.len())]
    Batch {
        failures: Vec<(PathBuf, Box<Error>)>,
    },
}

impl Error {
    /// Per-failure rendering of a `Batch` aggregate. Each item joins the
    /// path with the error and its full source chain so a reader sees the
    /// underlying cause without the renderer descending per-bullet.
    pub fn batch_bullets(&self) -> Option<Vec<String>> {
        match self {
            Self::Batch { failures } => Some(
                failures
                    .iter()
                    .map(|(path, err)| {
                        format!(
                            "{}: {}",
                            path.display(),
                            chain_layers(err.as_ref()).join(": ")
                        )
                    })
                    .collect(),
            ),
            _ => None,
        }
    }

    /// Per-error rendering of a `ValidationAfterEdit` aggregate. Returns
    /// `None` for non-aggregate variants.
    pub fn validation_bullets(&self) -> Option<Vec<String>> {
        match self {
            Self::ValidationAfterEdit(errs) => Some(errs.iter().map(|e| e.to_string()).collect()),
            _ => None,
        }
    }
}

/// Walk an error's source chain top-down, returning the `Display` of
/// each layer.
///
/// No dedup: every variant is either `#[error(transparent)]` or wraps
/// its source with distinct outer text, so adjacent layers never repeat.
/// A new `#[error("{0}")]` with `#[from]` would break this; fix at the
/// variant, not here.
pub fn chain_layers(err: &(dyn std::error::Error + 'static)) -> Vec<String> {
    let mut layers = vec![err.to_string()];
    let mut current = err.source();
    while let Some(source) = current {
        layers.push(source.to_string());
        current = source.source();
    }
    layers
}

/// Local result type for app-layer code.
pub type Result<T> = std::result::Result<T, Error>;
