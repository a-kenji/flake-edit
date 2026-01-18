use rnix::SyntaxKind;
use thiserror::Error;

/// Errors that can occur during AST walking and manipulation.
#[derive(Debug, Error)]
pub enum WalkerError {
    #[error("Expected root node, found {0:?}")]
    NotARoot(SyntaxKind),

    #[error("Expected {expected:?}, found {found:?}")]
    UnexpectedNodeKind {
        expected: SyntaxKind,
        found: SyntaxKind,
    },

    #[error("Feature not yet implemented: {0}")]
    NotImplemented(String),
}
