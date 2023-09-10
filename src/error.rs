use thiserror::Error;

#[derive(Debug, Error)]
pub enum FlkError {
    /// Io Error
    #[error("IoError: {0}")]
    Io(#[from] std::io::Error),
    // Reqwest Error
    // #[error("Incorrect Channel")]
    // IncorrectChannel(String),
}
