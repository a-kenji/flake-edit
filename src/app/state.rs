use std::path::PathBuf;

/// Application state for a flake-edit session.
///
/// Holds the flake content, file paths, and configuration options.
#[derive(Debug, Clone)]
pub struct AppState {
    /// Content of the flake.nix file
    pub flake_text: String,
    /// Path to the flake.nix file
    pub flake_path: PathBuf,
    /// Only show diff, don't write changes
    pub diff: bool,
    /// Skip running nix flake lock after changes
    pub no_lock: bool,
    /// Allow interactive TUI prompts
    pub interactive: bool,
}

impl AppState {
    pub fn new(flake_text: String, flake_path: PathBuf) -> Self {
        Self {
            flake_text,
            flake_path,
            diff: false,
            no_lock: false,
            interactive: true,
        }
    }

    pub fn with_diff(mut self, diff: bool) -> Self {
        self.diff = diff;
        self
    }

    pub fn with_no_lock(mut self, no_lock: bool) -> Self {
        self.no_lock = no_lock;
        self
    }

    pub fn with_interactive(mut self, interactive: bool) -> Self {
        self.interactive = interactive;
        self
    }
}
