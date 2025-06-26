use thiserror::Error;

#[derive(Debug, Error)]
pub enum FlakeEditError {
    #[error("IoError: {0}")]
    Io(#[from] std::io::Error),
    #[error("The flake should be a root.")]
    NotARoot,
    #[error("There is an error in the Lockfile: {0}")]
    LockError(String),
    #[error("Deserialization Error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error(
        "Input '{0}' already exists in the flake.\n\nTo replace it:\n  1. Remove it first: flake-edit remove {0}\n  2. Then add it again: flake-edit add {0} <flakeref>\n\nOr add it with a different [ID]:\n  flake-edit add [ID] <flakeref>\n\nTo see all current inputs: flake-edit list"
    )]
    DuplicateInput(String),
}
