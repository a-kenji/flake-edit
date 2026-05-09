use thiserror::Error;

/// Errors that can occur during AST walking and manipulation.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum WalkerError {
    /// rnix did not produce a parseable root for this input.
    #[error("flake.nix is not a parseable Nix file")]
    NotARoot,

    /// The top level of `flake.nix` contained something other than an
    /// `attr = value;` pair. `snippet` is a short excerpt of the offending
    /// node and `offset` is the byte offset where it starts.
    #[error("unexpected non-attribute at top level of flake.nix at byte {offset}: {snippet}")]
    UnexpectedTopLevel { snippet: String, offset: u32 },
}

impl WalkerError {
    pub(crate) fn unexpected_top_level(text: &str, offset: u32) -> Self {
        const MAX_SNIPPET: usize = 60;

        let single_line: String = text
            .replace(['\n', '\r'], " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let snippet = if single_line.chars().count() > MAX_SNIPPET {
            let truncated: String = single_line.chars().take(MAX_SNIPPET).collect();
            format!("{truncated}...")
        } else {
            single_line
        };
        Self::UnexpectedTopLevel { snippet, offset }
    }
}

#[cfg(test)]
mod tests {
    use super::WalkerError;

    #[test]
    fn unexpected_top_level_truncates_long_snippets() {
        let long = "x".repeat(200);
        let err = WalkerError::unexpected_top_level(&long, 7);
        let s = err.to_string();
        assert!(s.contains("..."), "long snippet should be truncated: {s}");
        assert!(s.contains("byte 7"), "byte offset should survive: {s}");
    }

    #[test]
    fn unexpected_top_level_collapses_newlines() {
        let err = WalkerError::unexpected_top_level("let\n  x = 1;\nin x", 3);
        let s = err.to_string();
        assert!(!s.contains('\n'), "newlines should be collapsed: {s:?}");
        assert!(s.ends_with("let x = 1; in x"));
    }
}
