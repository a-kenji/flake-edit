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

/// Generate the cache entry key from an id and uri.
///
/// The key format is `{id}.{uri}` which allows multiple URIs per input ID
/// (e.g., both a github: and path: URI for the same input).
fn entry_key(id: &str, uri: &str) -> String {
    format!("{}.{}", id, uri)
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

    /// Load the cache from the default location, or return a default empty cache.
    pub fn load() -> Self {
        Self::from_path(cache_file())
    }

    /// Load the cache from a specific file path, or return a default empty cache.
    pub fn from_path(path: &std::path::Path) -> Self {
        Self::try_from_path(path).unwrap_or_else(|e| {
            tracing::warn!("Could not read cache file {:?}: {}", path, e);
            Self::default()
        })
    }

    /// Try to load the cache from a specific file path.
    pub fn try_from_path(path: &std::path::Path) -> std::io::Result<Self> {
        let file = std::fs::File::open(path)?;
        serde_json::from_reader(file).map_err(|e| std::io::Error::other(e.to_string()))
    }

    /// Add or update a cache entry.
    pub fn add_entry(&mut self, id: String, uri: String) {
        let key = entry_key(&id, &uri);
        match self.entries.get_mut(&key) {
            Some(entry) => entry.hit += 1,
            None => {
                let entry = CacheEntry { id, uri, hit: 0 };
                self.entries.insert(key, entry);
            }
        }
    }

    /// List cached URIs sorted by hit count (most used first).
    pub fn list_uris(&self) -> Vec<String> {
        let mut entries: Vec<_> = self.entries.values().collect();
        entries.sort_by(|a, b| b.hit.cmp(&a.hit));
        entries.iter().map(|e| e.uri.clone()).collect()
    }

    /// List cached URIs for a specific input ID, sorted by hit count (most used first).
    ///
    /// This is useful for the "change" workflow where we want to suggest URIs
    /// that were previously used for the same input ID (e.g., both the remote
    /// github: URI and a local path: URI for testing).
    pub fn list_uris_for_id(&self, id: &str) -> Vec<String> {
        let mut entries: Vec<_> = self.entries.values().filter(|e| e.id == id).collect();
        entries.sort_by(|a, b| b.hit.cmp(&a.hit));
        entries.iter().map(|e| e.uri.clone()).collect()
    }

    /// Add entries for all inputs without incrementing hit counts.
    ///
    /// This is used to populate the cache with inputs discovered while
    /// running any command (list, change, update, etc.), not just add.
    /// Unlike `add_entry`, this does NOT increment hit counts for existing
    /// entries - it only adds new entries that don't exist yet.
    pub fn populate_from_inputs<'a>(&mut self, inputs: impl Iterator<Item = (&'a str, &'a str)>) {
        for (id, uri) in inputs {
            let key = entry_key(id, uri);
            // Only add if not already present (don't increment hit count)
            self.entries.entry(key).or_insert_with(|| CacheEntry {
                id: id.to_string(),
                uri: uri.to_string(),
                hit: 0,
            });
        }
    }
}

/// Populate the cache with inputs from a flake.
///
/// This is a convenience function that loads the cache, adds any new inputs,
/// and commits the changes. It's designed to be called from any command that
/// reads inputs, helping build up the cache over time.
///
/// Errors are logged but don't cause failures - caching is best-effort.
/// If `no_cache` is true, this function does nothing.
pub fn populate_cache_from_inputs<'a>(
    inputs: impl Iterator<Item = (&'a str, &'a str)>,
    no_cache: bool,
) {
    if no_cache {
        return;
    }

    let mut cache = Cache::load();
    let initial_len = cache.entries.len();
    cache.populate_from_inputs(inputs);

    // Only write if we added new entries
    if cache.entries.len() > initial_len
        && let Err(e) = cache.commit()
    {
        tracing::debug!("Could not write to cache: {}", e);
    }
}

/// Populate the cache from an InputMap.
///
/// Convenience wrapper around `populate_cache_from_inputs` for use with
/// the `FlakeEdit::list()` result. URIs are trimmed of surrounding quotes
/// since the raw syntax representation includes them.
/// If `no_cache` is true, this function does nothing.
pub fn populate_cache_from_input_map(inputs: &crate::edit::InputMap, no_cache: bool) {
    populate_cache_from_inputs(
        inputs
            .iter()
            .map(|(id, input)| (id.as_str(), input.url().trim_matches('"'))),
        no_cache,
    );
}

/// Default flake URI type prefixes for completion.
pub const DEFAULT_URI_TYPES: [&str; 14] = [
    "github:",
    "gitlab:",
    "sourcehut:",
    "git+https://",
    "git+ssh://",
    "git+http://",
    "git+file://",
    "git://",
    "path:",
    "file://",
    "tarball:",
    "https://",
    "http://",
    "flake:",
];

/// Configuration for cache usage.
///
/// Controls whether and where to read/write the URI completion cache.
#[derive(Debug, Clone, Default)]
pub enum CacheConfig {
    /// Use the default cache location (~/.local/share/flake-edit/)
    #[default]
    Default,
    /// Don't use any cache (for --no-cache flag)
    None,
    /// Use a custom cache file path (for --cache flag or testing)
    Custom(std::path::PathBuf),
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn test_list_uris_for_id() {
        let mut cache = Cache::default();
        // Add multiple URIs for the same ID (simulating local/remote toggle workflow)
        cache.add_entry("treefmt-nix".into(), "github:numtide/treefmt-nix".into());
        cache.add_entry(
            "treefmt-nix".into(),
            "path:/home/user/dev/treefmt-nix".into(),
        );
        // Add unrelated entry
        cache.add_entry("nixpkgs".into(), "github:NixOS/nixpkgs".into());
        // Increment hit on the github one
        cache.add_entry("treefmt-nix".into(), "github:numtide/treefmt-nix".into());

        let uris = cache.list_uris_for_id("treefmt-nix");
        assert_eq!(uris.len(), 2);
        // github should be first due to higher hit count
        assert_eq!(uris[0], "github:numtide/treefmt-nix");
        assert_eq!(uris[1], "path:/home/user/dev/treefmt-nix");

        // Should not include nixpkgs
        assert!(!uris.contains(&"github:NixOS/nixpkgs".to_string()));
    }

    #[test]
    fn test_list_uris_for_id_empty() {
        let cache = Cache::default();
        let uris = cache.list_uris_for_id("nonexistent");
        assert!(uris.is_empty());
    }

    #[test]
    fn test_populate_from_inputs() {
        let mut cache = Cache::default();

        // Add some initial entries
        cache.add_entry("nixpkgs".into(), "github:NixOS/nixpkgs".into());
        cache.add_entry("nixpkgs".into(), "github:NixOS/nixpkgs".into()); // hit = 1

        // Populate with inputs (simulating what happens when running a command)
        let inputs = vec![
            ("nixpkgs", "github:NixOS/nixpkgs"),           // Already exists
            ("flake-utils", "github:numtide/flake-utils"), // New
            ("home-manager", "github:nix-community/home-manager"), // New
        ];
        cache.populate_from_inputs(inputs.into_iter());

        // Should have 3 entries total
        let uris = cache.list_uris();
        assert_eq!(uris.len(), 3);

        // nixpkgs should still be first (hit=1, others hit=0)
        assert_eq!(uris[0], "github:NixOS/nixpkgs");

        // New entries should exist
        assert!(uris.contains(&"github:numtide/flake-utils".to_string()));
        assert!(uris.contains(&"github:nix-community/home-manager".to_string()));
    }

    #[test]
    fn test_populate_does_not_increment_hits() {
        let mut cache = Cache::default();

        // Add entry with hit count
        cache.add_entry("nixpkgs".into(), "github:NixOS/nixpkgs".into());
        cache.add_entry("nixpkgs".into(), "github:NixOS/nixpkgs".into()); // hit = 1

        // Populate with same entry
        let inputs = vec![("nixpkgs", "github:NixOS/nixpkgs")];
        cache.populate_from_inputs(inputs.into_iter());

        // Hit count should still be 1, not 2
        let entry = cache.entries.get("nixpkgs.github:NixOS/nixpkgs").unwrap();
        assert_eq!(entry.hit, 1);
    }
}
