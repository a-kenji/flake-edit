use thiserror::Error;

#[derive(Debug, Error)]
pub enum FeError {
    #[allow(unused)]
    #[error("Error: {0}")]
    Error(String),
    #[error("IoError: {0}")]
    Io(#[from] std::io::Error),
    #[error("FlakeEdit: {0}")]
    FlakeEdit(#[from] flake_edit::error::FlakeEditError),
}
