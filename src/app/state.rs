use std::path::PathBuf;

use crate::cache::CacheConfig;

/// Application state for a flake-edit session.
///
/// Holds the flake content, file paths, and configuration options.
#[derive(Debug, Clone)]
pub struct AppState {
    /// Content of the flake.nix file
    pub flake_text: String,
    /// Path to the flake.nix file
    pub flake_path: PathBuf,
    /// Path to the flake.lock file (if specified)
    pub lock_file: Option<PathBuf>,
    /// Only show diff, don't write changes
    pub diff: bool,
    /// Skip running nix flake lock after changes
    pub no_lock: bool,
    /// Allow interactive TUI prompts
    pub interactive: bool,
    /// Disable reading from and writing to the completion cache
    pub no_cache: bool,
    /// Custom cache file path (for testing or portable configs)
    pub cache_path: Option<PathBuf>,
}

impl AppState {
    pub fn new(flake_text: String, flake_path: PathBuf) -> Self {
        Self {
            flake_text,
            flake_path,
            lock_file: None,
            diff: false,
            no_lock: false,
            interactive: true,
            no_cache: false,
            cache_path: None,
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

    pub fn with_lock_file(mut self, lock_file: Option<PathBuf>) -> Self {
        self.lock_file = lock_file;
        self
    }

    pub fn with_no_cache(mut self, no_cache: bool) -> Self {
        self.no_cache = no_cache;
        self
    }

    pub fn with_cache_path(mut self, cache_path: Option<PathBuf>) -> Self {
        self.cache_path = cache_path;
        self
    }

    /// Get the cache configuration based on CLI flags.
    pub fn cache_config(&self) -> CacheConfig {
        if self.no_cache {
            CacheConfig::None
        } else if let Some(ref path) = self.cache_path {
            CacheConfig::Custom(path.clone())
        } else {
            CacheConfig::Default
        }
    }
}
