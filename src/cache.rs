use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

static CACHE_FILE_NAME: &str = "flake_edit.json";

fn cache_dir() -> &'static PathBuf {
    static CACHE_DIR: OnceLock<PathBuf> = OnceLock::new();
    CACHE_DIR.get_or_init(|| {
        let project_dir = ProjectDirs::from("com", "a-kenji", "flake-edit").unwrap();
        project_dir.data_dir().to_path_buf()
    })
}

fn cache_file() -> &'static PathBuf {
    static CACHE_FILE: OnceLock<PathBuf> = OnceLock::new();
    CACHE_FILE.get_or_init(|| cache_dir().join(CACHE_FILE_NAME))
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct CacheEntry {
    id: String,
    uri: String,
    hit: u32,
}

/// Cache for storing previously used flake URIs.
///
/// Used for shell completions to suggest frequently used inputs.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Cache {
    entries: HashMap<String, CacheEntry>,
}

impl Cache {
    /// Save the cache to disk.
    pub fn commit(&self) -> std::io::Result<()> {
        let cache_dir = cache_dir();
        if !cache_dir.exists() {
            std::fs::create_dir_all(cache_dir)?;
        }
        let cache_file_location = cache_file();
        let cache_file = std::fs::File::create(cache_file_location)?;
        serde_json::to_writer(cache_file, self)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        Ok(())
    }

    /// Load the cache from disk, or return a default empty cache.
    pub fn load() -> Self {
        Self::try_load().unwrap_or_else(|e| {
            tracing::warn!("Could not read cache file: {}", e);
            Self::default()
        })
    }

    /// Try to load the cache from disk.
    pub fn try_load() -> std::io::Result<Self> {
        let file = std::fs::File::open(cache_file())?;
        serde_json::from_reader(file).map_err(|e| std::io::Error::other(e.to_string()))
    }

    /// Add or update a cache entry.
    pub fn add_entry(&mut self, id: String, uri: String) {
        let entry_id = format!("{}.{}", id, uri);
        match self.entries.get_mut(&entry_id) {
            Some(entry) => entry.hit += 1,
            None => {
                let entry = CacheEntry { id, uri, hit: 0 };
                self.entries.insert(entry_id, entry);
            }
        }
    }

    /// List cached URIs sorted by hit count (most used first).
    pub fn list_uris(&self) -> Vec<String> {
        let mut entries: Vec<_> = self.entries.values().collect();
        entries.sort_by(|a, b| b.hit.cmp(&a.hit));
        entries.iter().map(|e| e.uri.clone()).collect()
    }
}

/// Default flake URI type prefixes for completion.
pub const DEFAULT_URI_TYPES: [&str; 7] = [
    "github:",
    "gitlab:",
    "sourcehut:",
    "git+https://",
    "git+ssh://",
    "path:",
    "tarball:",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_dir() {
        let _cache_dir = cache_dir();
    }

    #[test]
    fn test_cache_add_and_list() {
        let mut cache = Cache::default();
        cache.add_entry("nixpkgs".into(), "github:NixOS/nixpkgs".into());
        cache.add_entry(
            "home-manager".into(),
            "github:nix-community/home-manager".into(),
        );
        cache.add_entry("nixpkgs".into(), "github:NixOS/nixpkgs".into()); // Increment hit

        let uris = cache.list_uris();
        assert_eq!(uris.len(), 2);
        // nixpkgs should be first due to higher hit count
        assert_eq!(uris[0], "github:NixOS/nixpkgs");
    }
}
