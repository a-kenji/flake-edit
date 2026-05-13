//! Persistent, ETag-backed cache for forge HTTP responses.
//!
//! `HttpCache` wraps an [`HttpClient`] and intercepts `get` calls.
//! For each `(url, auth)` pair it remembers the ETag the forge
//! handed out last time; on the next call it sends
//! `If-None-Match: <etag>`. A `304 Not Modified` returns the
//! cached body without burning the anonymous rate-limit quota; a
//! `200` overwrites the cached body and ETag.
//!
//! The cache is XDG-located and persists across `flake-edit`
//! invocations; `rm` the file to invalidate it.
//!
//! Any cache failure (unreadable file, unparseable payload,
//! unwritable directory) degrades to "no cache".
//!
//! Concurrency: one `HttpCache` instance owns one mutex, so a
//! worker pool fanning out requests against a single
//! [`super::api::ForgeClient`] serializes its mutations cleanly.
//! Multiple `HttpCache` instances pointed at the same file
//! across separate processes race on the atomic rename; the last
//! writer wins.

use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

use directories::ProjectDirs;
use serde::Deserialize;
use serde::Serialize;

use super::api::ApiError;
use super::api::ConditionalResponse;
use super::api::Headers;
use super::api::HttpClient;

/// On-disk schema version. Bump whenever the layout of `Entry` or
/// the response-parsing pipeline changes in a way that makes old
/// cached bodies unsafe to reuse.
const SCHEMA_VERSION: u32 = 1;
const CACHE_FILE_NAME: &str = "forge_http_cache.json";

/// Hard cap on resident entries. The natural upper bound on a
/// typical flake's distinct `(url, auth)` lookups is a few dozen.
/// The cap exists to keep a long-lived `~/.cache` directory from
/// accreting an unbounded record of every flake the user has
/// ever updated.
const MAX_ENTRIES: usize = 1024;

/// Age cutoff for resident entries. The cache loses no
/// correctness when an entry expires: at worst we re-issue an
/// unconditional fetch and repopulate. The cap bounds growth for
/// flakes that churn through inputs.
const MAX_ENTRY_AGE_SECS: u64 = 30 * 24 * 60 * 60;

#[derive(Serialize, Deserialize, Default)]
struct FileFormat {
    schema_version: u32,
    entries: HashMap<String, Entry>,
}

/// Cache record for a single `(url, auth)` pair. `deny_unknown_fields`
/// is a tripwire: when the on-disk shape evolves without a
/// corresponding `SCHEMA_VERSION` bump, parsing fails loudly and the
/// cache is treated as empty rather than silently dropping renamed
/// or removed fields.
#[derive(Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
struct Entry {
    /// Plaintext URL, kept for debuggability of the cache file.
    /// The lookup key is the hash that prefixes it. Anything in the
    /// URL (owner/repo, query-string parameters) lives in the cache
    /// file in clear; the file is XDG-scoped to the current user and
    /// holds no other secrets.
    url: String,
    etag: String,
    body: String,
    /// Unix seconds, used for age-based eviction.
    stored_at: u64,
}

struct State {
    entries: HashMap<String, Entry>,
    dirty: bool,
}

/// HTTP-layer wrapper that turns repeat requests into 304-revalidated
/// reads against a persistent on-disk cache.
pub(crate) struct HttpCache {
    inner: HttpClient,
    state: Mutex<State>,
    /// `None` when persistence is disabled (env var set, or
    /// directories-crate could not resolve the XDG cache root).
    persist_path: Option<PathBuf>,
}

impl HttpCache {
    /// Build the default XDG-located cache around a fresh
    /// [`HttpClient`]. Persistence silently degrades to in-memory
    /// when the XDG cache root cannot be resolved.
    pub(crate) fn new() -> Self {
        Self::with_path(default_cache_path())
    }

    /// Construct with an explicit path. `None` disables persistence;
    /// `Some` reads the file if present, ignoring parse failures.
    fn with_path(path: Option<PathBuf>) -> Self {
        let entries = match path.as_deref() {
            Some(p) => load_entries(p),
            None => HashMap::new(),
        };
        let entries = prune_by_age(entries);
        Self {
            inner: HttpClient::default(),
            state: Mutex::new(State {
                entries,
                dirty: false,
            }),
            persist_path: path,
        }
    }

    /// Partition the cache by authorization header so a response
    /// fetched with token A is never served to a request using token
    /// B (or no token at all). The token itself is hashed and never
    /// lands in the cache file.
    fn cache_key(url: &str, headers: &Headers) -> String {
        use std::hash::Hash;
        use std::hash::Hasher;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        headers
            .authorization
            .as_deref()
            .unwrap_or("")
            .hash(&mut hasher);
        format!("{:016x}|{}", hasher.finish(), url)
    }

    /// Silently swallows errors; this runs from `Drop` and must not
    /// panic.
    fn flush(&self) {
        let Some(ref path) = self.persist_path else {
            return;
        };
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if !state.dirty {
            return;
        }
        let entries = prune_by_size(state.entries.clone());
        let file = FileFormat {
            schema_version: SCHEMA_VERSION,
            entries,
        };
        match write_atomically(path, &file) {
            Ok(()) => state.dirty = false,
            Err(e) => tracing::debug!("forge cache: write failed: {}", e),
        }
    }
}

impl Drop for HttpCache {
    fn drop(&mut self) {
        self.flush();
    }
}

impl HttpCache {
    pub(crate) fn get(&self, url: &str, headers: &Headers) -> Result<String, ApiError> {
        let key = Self::cache_key(url, headers);

        let known_etag = self
            .state
            .lock()
            .ok()
            .and_then(|s| s.entries.get(&key).map(|e| e.etag.clone()));

        let etag_ref = if self.persist_path.is_some() {
            known_etag.as_deref()
        } else {
            None
        };

        match self.inner.get_conditional(url, headers, etag_ref)? {
            ConditionalResponse::NotModified => {
                let body = self
                    .state
                    .lock()
                    .ok()
                    .and_then(|s| s.entries.get(&key).map(|e| e.body.clone()));
                match body {
                    Some(b) => Ok(b),
                    None => {
                        // The server claims our cache is valid but
                        // our cache is empty. Fall back to a normal
                        // fetch; the inner requester will likely
                        // serve the real body now.
                        tracing::debug!("forge cache: 304 with no cached body, refetching");
                        self.inner.get(url, headers)
                    }
                }
            }
            ConditionalResponse::Body {
                body,
                etag: Some(new_etag),
            } if self.persist_path.is_some() => {
                if let Ok(mut state) = self.state.lock() {
                    state.entries.insert(
                        key,
                        Entry {
                            url: url.to_string(),
                            etag: new_etag,
                            body: body.clone(),
                            stored_at: now_secs(),
                        },
                    );
                    state.dirty = true;
                }
                Ok(body)
            }
            ConditionalResponse::Body { body, .. } => Ok(body),
        }
    }

    pub(crate) fn head_status(&self, url: &str, headers: &Headers) -> Result<bool, ApiError> {
        // HEAD probes are tiny and the forge does not hand out
        // ETags for the dedicated `/branches/<branch>` endpoint we
        // probe. Pass through unmodified.
        self.inner.head_status(url, headers)
    }

    pub(crate) fn post_json(
        &self,
        url: &str,
        headers: &Headers,
        body: &str,
    ) -> Result<String, ApiError> {
        // GraphQL POSTs cannot be cached by URL alone: the response
        // body depends on the request body. Pass through; the
        // persistent cache only covers GET responses.
        self.inner.post_json(url, headers, body)
    }
}

fn default_cache_path() -> Option<PathBuf> {
    ProjectDirs::from("com", "a-kenji", "flake-edit").map(|p| p.cache_dir().join(CACHE_FILE_NAME))
}

fn load_entries(path: &Path) -> HashMap<String, Entry> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return HashMap::new(),
    };
    match serde_json::from_reader::<_, FileFormat>(file) {
        Ok(parsed) if parsed.schema_version == SCHEMA_VERSION => parsed.entries,
        Ok(_) => {
            tracing::debug!("forge cache: schema version mismatch, ignoring");
            HashMap::new()
        }
        Err(e) => {
            tracing::debug!("forge cache: parse failed: {}", e);
            HashMap::new()
        }
    }
}

fn prune_by_age(mut entries: HashMap<String, Entry>) -> HashMap<String, Entry> {
    let cutoff = now_secs().saturating_sub(MAX_ENTRY_AGE_SECS);
    entries.retain(|_, e| e.stored_at >= cutoff);
    entries
}

fn prune_by_size(mut entries: HashMap<String, Entry>) -> HashMap<String, Entry> {
    if entries.len() <= MAX_ENTRIES {
        return entries;
    }
    let mut by_age: Vec<(String, u64)> = entries
        .iter()
        .map(|(k, e)| (k.clone(), e.stored_at))
        .collect();
    by_age.sort_by_key(|(_, t)| *t);
    let to_drop = entries.len() - MAX_ENTRIES;
    for (k, _) in by_age.into_iter().take(to_drop) {
        entries.remove(&k);
    }
    entries
}

fn write_atomically(path: &Path, file: &FileFormat) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut tmp = path.to_path_buf();
    // Same directory as the final file so the rename is atomic on
    // every platform we target.
    tmp.set_extension("json.tmp");
    {
        let f = std::fs::File::create(&tmp)?;
        serde_json::to_writer(f, file).map_err(std::io::Error::other)?;
    }
    std::fs::rename(&tmp, path)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir() -> tempfile::TempDir {
        tempfile::tempdir().expect("create tempdir")
    }

    fn sample_entry(suffix: &str, stored_at: u64) -> Entry {
        Entry {
            url: format!("https://api.example/r-{suffix}"),
            etag: format!("etag-{suffix}"),
            body: format!("BODY-{suffix}"),
            stored_at,
        }
    }

    #[test]
    fn round_trip_through_disk_preserves_entries() {
        // Write a file through the real persistence path, then reload
        // it and confirm `load_entries` returns the same shape.
        let dir = tempdir();
        let path = dir.path().join("c.json");
        let mut entries = HashMap::new();
        entries.insert("k0".to_string(), sample_entry("0", 100));
        entries.insert("k1".to_string(), sample_entry("1", 200));
        let file = FileFormat {
            schema_version: SCHEMA_VERSION,
            entries: entries.clone(),
        };
        write_atomically(&path, &file).expect("write");

        let loaded = load_entries(&path);
        assert_eq!(loaded.len(), 2);
        assert_eq!(
            loaded.get("k0").map(|e| &e.body),
            Some(&"BODY-0".to_string())
        );
        assert_eq!(
            loaded.get("k1").map(|e| &e.etag),
            Some(&"etag-1".to_string())
        );
    }

    #[test]
    fn corrupt_cache_file_loads_empty() {
        let dir = tempdir();
        let path = dir.path().join("c.json");
        std::fs::write(&path, b"{ this is not json ").unwrap();
        assert!(load_entries(&path).is_empty());
    }

    #[test]
    fn schema_version_mismatch_loads_empty() {
        let dir = tempdir();
        let path = dir.path().join("c.json");
        std::fs::write(
            &path,
            br#"{"schema_version":9999,"entries":{"k":{"url":"u","etag":"e","body":"old","stored_at":0}}}"#,
        )
        .unwrap();
        assert!(load_entries(&path).is_empty());
    }

    #[test]
    fn deny_unknown_fields_rejects_drift() {
        // `Entry` is `deny_unknown_fields`, so silently adding a new
        // on-disk field without bumping `SCHEMA_VERSION` must fail
        // the load instead of dropping data on next write.
        let dir = tempdir();
        let path = dir.path().join("c.json");
        std::fs::write(
            &path,
            br#"{"schema_version":1,"entries":{"k":{"url":"u","etag":"e","body":"b","stored_at":0,"surprise":"!"}}}"#,
        )
        .unwrap();
        assert!(load_entries(&path).is_empty());
    }

    #[test]
    fn cache_key_isolates_by_authorization() {
        let url = "https://api.example/r";
        let no_auth = Headers {
            user_agent: None,
            authorization: None,
        };
        let with_a = Headers {
            user_agent: None,
            authorization: Some("Bearer token-a".to_string()),
        };
        let with_b = Headers {
            user_agent: None,
            authorization: Some("Bearer token-b".to_string()),
        };

        let k_none = HttpCache::cache_key(url, &no_auth);
        let k_a = HttpCache::cache_key(url, &with_a);
        let k_b = HttpCache::cache_key(url, &with_b);

        assert_ne!(
            k_none, k_a,
            "no-auth and token-a must hash to distinct slots"
        );
        assert_ne!(k_a, k_b, "two distinct tokens must hash to distinct slots");
        assert_eq!(
            k_a,
            HttpCache::cache_key(url, &with_a),
            "same (url, auth) must hash to the same slot"
        );
        assert!(
            !k_a.contains("token-a"),
            "raw token must never appear in the cache key: {k_a}"
        );
    }

    #[test]
    fn prune_by_age_drops_entries_older_than_cutoff() {
        let now = now_secs();
        let cutoff = MAX_ENTRY_AGE_SECS;
        let mut entries = HashMap::new();
        entries.insert("fresh".to_string(), sample_entry("fresh", now));
        entries.insert(
            "stale".to_string(),
            sample_entry("stale", now.saturating_sub(cutoff + 60)),
        );
        let pruned = prune_by_age(entries);
        assert!(pruned.contains_key("fresh"));
        assert!(!pruned.contains_key("stale"));
    }

    #[test]
    fn prune_by_size_keeps_newest_when_over_cap() {
        let mut entries = HashMap::new();
        // Generate MAX_ENTRIES + 5 entries with distinct stored_at, so
        // the oldest five must be dropped.
        for i in 0..(MAX_ENTRIES + 5) {
            entries.insert(format!("k{i}"), sample_entry(&format!("{i}"), i as u64));
        }
        let pruned = prune_by_size(entries);
        assert_eq!(pruned.len(), MAX_ENTRIES);
        // The five with the smallest stored_at (`k0`..=`k4`) must be gone.
        for i in 0..5 {
            assert!(
                !pruned.contains_key(&format!("k{i}")),
                "k{i} should be evicted"
            );
        }
        // The newest entry (`k{MAX_ENTRIES + 4}`) must survive.
        assert!(pruned.contains_key(&format!("k{}", MAX_ENTRIES + 4)));
    }

    #[test]
    fn write_atomically_creates_parent_directories() {
        let dir = tempdir();
        let path = dir.path().join("nested").join("deeper").join("c.json");
        let file = FileFormat {
            schema_version: SCHEMA_VERSION,
            entries: HashMap::new(),
        };
        write_atomically(&path, &file).expect("write should mkdir -p the parent");
        assert!(path.exists());
    }

    #[test]
    fn write_atomically_fails_when_parent_is_a_file() {
        // The default path under a regular file cannot be created.
        // The function must surface the error, not panic.
        let dir = tempdir();
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"file, not a directory").unwrap();
        let path = blocker.join("c.json");
        let file = FileFormat {
            schema_version: SCHEMA_VERSION,
            entries: HashMap::new(),
        };
        assert!(write_atomically(&path, &file).is_err());
    }
}
