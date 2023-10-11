use thiserror::Error;

#[derive(Debug, Error)]
pub enum FlakeEditError {
    /// Io Error
    #[error("IoError: {0}")]
    Io(#[from] std::io::Error),
    #[error("The flake should be a root.")]
    NotARoot,
    // Reqwest Error
    // #[error("Incorrect Channel")]
    // IncorrectChannel(String),
}
