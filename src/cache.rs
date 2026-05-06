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

/// Build the cache entry key as `{id}.{uri}`. Keying by `(id, uri)` allows
/// multiple URIs per input id (e.g. both a `github:` and a `path:` URI).
fn entry_key(id: &str, uri: &str) -> String {
    format!("{}.{}", id, uri)
}

/// Persistent store of previously seen flake URIs.
///
/// Powers shell-completion suggestions, ranked by hit count.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Cache {
    entries: HashMap<String, CacheEntry>,
}

impl Cache {
    /// Write the cache to its on-disk location, creating the parent directory
    /// if needed.
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

    /// Load the cache from the default location, or return an empty cache on
    /// any failure.
    pub fn load() -> Self {
        Self::from_path(cache_file())
    }

    /// Load the cache from `path`, or return an empty cache on any failure.
    pub fn from_path(path: &std::path::Path) -> Self {
        Self::try_from_path(path).unwrap_or_else(|e| {
            tracing::warn!("Could not read cache file {:?}: {}", path, e);
            Self::default()
        })
    }

    /// Load the cache from `path`, surfacing read or parse errors.
    ///
    /// # Errors
    ///
    /// Returns [`std::io::Error`] if `path` cannot be opened or its JSON
    /// payload cannot be deserialized.
    pub fn try_from_path(path: &std::path::Path) -> std::io::Result<Self> {
        let file = std::fs::File::open(path)?;
        serde_json::from_reader(file).map_err(|e| std::io::Error::other(e.to_string()))
    }

    /// Insert or bump the hit count of the `(id, uri)` entry.
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

    /// All cached URIs sorted by descending hit count.
    pub fn list_uris(&self) -> Vec<String> {
        let mut entries: Vec<_> = self.entries.values().collect();
        entries.sort_by(|a, b| b.hit.cmp(&a.hit));
        entries.iter().map(|e| e.uri.clone()).collect()
    }

    /// Cached URIs for `id` sorted by descending hit count.
    ///
    /// Useful for the `change` workflow, which suggests URIs that have been
    /// used for the same input id (e.g. both a remote `github:` and a local
    /// `path:` URI for testing).
    pub fn list_uris_for_id(&self, id: &str) -> Vec<String> {
        let mut entries: Vec<_> = self.entries.values().filter(|e| e.id == id).collect();
        entries.sort_by(|a, b| b.hit.cmp(&a.hit));
        entries.iter().map(|e| e.uri.clone()).collect()
    }

    /// Insert any `(id, uri)` pairs not already present, without bumping hit
    /// counts on existing entries.
    ///
    /// Use this when populating the cache as a side effect of any command
    /// that reads inputs (`list`, `change`, `update`, ...), not only `add`.
    pub fn populate_from_inputs<'a>(&mut self, inputs: impl Iterator<Item = (&'a str, &'a str)>) {
        for (id, uri) in inputs {
            let key = entry_key(id, uri);
            self.entries.entry(key).or_insert_with(|| CacheEntry {
                id: id.to_string(),
                uri: uri.to_string(),
                hit: 0,
            });
        }
    }
}

/// Load the on-disk cache, add any new `(id, uri)` pairs, and commit.
///
/// Best-effort: I/O failures are logged, not propagated. A `no_cache` of
/// `true` makes the call a no-op.
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

    if cache.entries.len() > initial_len
        && let Err(e) = cache.commit()
    {
        tracing::debug!("Could not write to cache: {}", e);
    }
}

/// Convenience wrapper over [`populate_cache_from_inputs`] for the result of
/// [`crate::edit::FlakeEdit::list`]. A `no_cache` of `true` makes the call a
/// no-op.
pub fn populate_cache_from_input_map(inputs: &crate::edit::InputMap, no_cache: bool) {
    populate_cache_from_inputs(
        inputs.iter().map(|(id, input)| (id.as_str(), input.url())),
        no_cache,
    );
}

/// Flake URI type prefixes offered by completion.
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

/// Where to read and write the URI completion cache.
#[derive(Debug, Clone, Default)]
pub enum CacheConfig {
    /// Default XDG location (`~/.local/share/flake-edit/`).
    #[default]
    Default,
    /// Disable caching entirely (`--no-cache`).
    None,
    /// Read and write at a custom path (`--cache`, or tests).
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
