use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

static CACHE_FILE_NAME: &str = "fe.json";

fn cache_dir() -> &'static PathBuf {
    static CACHE_DIR: OnceLock<PathBuf> = OnceLock::new();
    CACHE_DIR.get_or_init(|| {
        let project_dir = ProjectDirs::from("com", "a-kenji", "fe").unwrap();
        return project_dir.data_dir().to_path_buf();
    })
}

fn cache_file() -> &'static PathBuf {
    static CACHE_FILE: OnceLock<PathBuf> = OnceLock::new();
    CACHE_FILE.get_or_init(|| cache_dir().join(CACHE_FILE_NAME))
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub(crate) struct FeCacheEntry {
    id: String,
    uri: String,
    // how many times the entry was used
    hit: u32,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct FeCache {
    entries: HashMap<String, FeCacheEntry>,
}

impl FeCache {
    pub(crate) fn commit(&self) -> anyhow::Result<()> {
        let cache_dir = cache_dir();
        if !cache_dir.exists() {
            std::fs::create_dir_all(cache_dir)?;
        }
        let cache_file_location = cache_file();
        let cache_file = std::fs::File::create(cache_file_location)?;
        serde_json::to_writer(cache_file, self)?;
        Ok(())
    }

    pub(crate) fn get_or_init(&self) -> Self {
        self.get()
            .or_else(|e| {
                tracing::warn!("Could not read cache file: {}", e);
                let fe_cache: Result<Self, anyhow::Error> = Ok(Self::default());
                fe_cache
            })
            .unwrap()
    }

    pub(crate) fn get(&self) -> anyhow::Result<Self> {
        Ok(std::fs::File::open(cache_file())
            .and_then(|file| serde_json::from_reader(file).map_err(|e| e.into()))?)
    }

    pub(crate) fn add_entry(&mut self, id: String, uri: String) {
        let entry_id = format!("{}.{}", id, uri);
        match self.entries.get_mut(&entry_id) {
            Some(entry) => entry.hit += 1,
            None => {
                let entry = FeCacheEntry { id, uri, hit: 0 };
                self.entries.insert(entry_id, entry);
            }
        }
    }
    /// Used for shell completions.
    /// Should be sorted by hit count.
    pub(crate) fn list(&mut self) -> Vec<String> {
        let mut res = Vec::new();
        let mut entries: Vec<_> = self.entries.values().collect();
        entries.sort_by(|a, b| b.hit.cmp(&a.hit));
        for entry in entries {
            res.push(entry.uri.clone());
        }
        res
    }
}

pub const fn default_types() -> [&'static str; 2] {
    ["github", "gitlab"]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_cache_dir() {
        let _cache_dir = cache_dir();
    }
}
