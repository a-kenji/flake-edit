use thiserror::Error;

#[derive(Debug, Error)]
pub enum FeError {
    /// Generic fe error.
    #[allow(unused)]
    #[error("Error: {0}")]
    Error(String),
    /// Io Error
    #[error("IoError: {0}")]
    Io(#[from] std::io::Error),
    // Reqwest Error
    // #[error("Incorrect Channel")]
    // IncorrectChannel(String),
    #[error("FlakeEdit: {0}")]
    FlakeEdit(#[from] flake_edit::error::FlakeEditError),
}
