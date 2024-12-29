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
}
