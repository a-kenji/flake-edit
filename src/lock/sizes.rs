//! Store-size estimation for flake inputs.
//!
//! `flake-edit follow --stats` reports how much disk follows deduplication saves.
use std::collections::{BTreeSet, HashMap};
use std::process::{Command, Stdio};
use std::time::Duration;

use base64::Engine as _;
use sha2::{Digest, Sha256};

use crate::follows::{AttrPath, Segment, join_raw};
use crate::lock::{FlakeLock, NestedInput};

/// Wall-clock cap a single `nix path-info` invocation.
const PATH_INFO_TIMEOUT: Duration = Duration::from_secs(30);

/// Sizes and store paths for the attribute paths of a lockfile.
#[derive(Debug, Default)]
pub struct InputSizes {
    /// Dotted attribute path to the store path its lock node unpacks to.
    store_paths: HashMap<String, String>,
    /// Store path to NAR size in bytes, where known.
    sizes: HashMap<String, u64>,
}

impl InputSizes {
    pub fn query(lock: &FlakeLock, nested_inputs: &[NestedInput]) -> Self {
        Self::query_with(lock, nested_inputs, query_path_sizes)
    }

    fn query_with(
        lock: &FlakeLock,
        nested_inputs: &[NestedInput],
        size_lookup: impl FnOnce(&[&str]) -> HashMap<String, u64>,
    ) -> Self {
        let store_dir = store_dir();
        let mut store_paths: HashMap<String, String> = HashMap::new();
        for (attr, node_key) in attr_to_node_map(lock, nested_inputs) {
            let nar_hash = lock
                .nodes
                .get(&node_key)
                .and_then(|node| node.locked.as_ref())
                .and_then(|locked| locked.nar_hash.as_deref());
            let Some(store_path) =
                nar_hash.and_then(|hash| store_path_from_nar_hash(&store_dir, hash))
            else {
                continue;
            };
            store_paths.insert(attr, store_path);
        }
        let unique: BTreeSet<&str> = store_paths.values().map(String::as_str).collect();
        let sizes = if unique.is_empty() {
            HashMap::new()
        } else {
            let paths: Vec<&str> = unique.into_iter().collect();
            size_lookup(&paths)
        };
        Self { store_paths, sizes }
    }

    pub fn from_attr_entries(
        entries: impl IntoIterator<Item = (String, String, Option<u64>)>,
    ) -> Self {
        let mut store_paths = HashMap::new();
        let mut sizes = HashMap::new();
        for (attr, store_path, size) in entries {
            if let Some(size) = size {
                sizes.insert(store_path.clone(), size);
            }
            store_paths.insert(attr, store_path);
        }
        Self { store_paths, sizes }
    }

    /// NAR size in bytes of the store path `path` resolves to.
    pub fn size_for(&self, path: &AttrPath) -> Option<u64> {
        self.sizes.get(self.store_path_for(path)?).copied()
    }

    /// Store path `path` resolves to.
    pub fn store_path_for(&self, path: &AttrPath) -> Option<&str> {
        self.store_paths.get(&dotted(path)).map(String::as_str)
    }

    /// Shortest attribute path resolving to `store_path`.
    pub fn label_for_store_path(&self, store_path: &str) -> Option<&str> {
        self.store_paths
            .iter()
            .filter(|(_, sp)| sp.as_str() == store_path)
            .map(|(attr, _)| attr.as_str())
            .min_by_key(|attr| (attr.matches('.').count(), attr.len(), *attr))
    }
}

/// Dotted raw form of an attribute path (`hls-1.10.nixpkgs`).
fn dotted(path: &AttrPath) -> String {
    join_raw(path.segments())
}

fn store_dir() -> String {
    std::env::var("NIX_STORE_DIR").unwrap_or_else(|_| "/nix/store".to_string())
}

fn attr_to_node_map(lock: &FlakeLock, nested_inputs: &[NestedInput]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    if let Some(root_inputs) = lock
        .nodes
        .get(&lock.root)
        .and_then(|node| node.inputs.as_ref())
    {
        for name in root_inputs.keys() {
            let Ok(seg) = Segment::from_unquoted(name.clone()) else {
                continue;
            };
            if let Ok(key) = lock.resolve_input_path(&AttrPath::new(seg)) {
                out.insert(name.clone(), key);
            }
        }
    }
    for nested in nested_inputs {
        if let Ok(key) = lock.resolve_input_path(&nested.path) {
            out.insert(dotted(&nested.path), key);
        }
    }
    out
}

/// Compute the store path a fixed-output source tree with `nar_hash`
/// unpacks to.
///
/// Returns `None` for hashes that are not SRI sha256.
fn store_path_from_nar_hash(store_dir: &str, nar_hash: &str) -> Option<String> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let encoded = nar_hash.strip_prefix("sha256-")?;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    if raw.len() != 32 {
        return None;
    }
    let mut hex = String::with_capacity(64);
    for byte in &raw {
        hex.push(HEX[(byte >> 4) as usize] as char);
        hex.push(HEX[(byte & 0x0f) as usize] as char);
    }
    let fingerprint = format!("source:sha256:{hex}:{store_dir}:source");
    let digest = Sha256::digest(fingerprint.as_bytes());
    let mut folded = [0u8; 20];
    for (i, byte) in digest.iter().enumerate() {
        folded[i % 20] ^= byte;
    }
    Some(format!("{store_dir}/{}-source", nix_base32(&folded)))
}

fn nix_base32(bytes: &[u8; 20]) -> String {
    const ALPHABET: &[u8; 32] = b"0123456789abcdfghijklmnpqrsvwxyz";
    let mut out = String::with_capacity(32);
    for n in (0..32).rev() {
        let bit = n * 5;
        let byte = bit / 8;
        let shift = bit % 8;
        let mut value = bytes[byte] >> shift;
        if shift > 0 && byte + 1 < bytes.len() {
            value |= bytes[byte + 1] << (8 - shift);
        }
        out.push(ALPHABET[(value & 0x1f) as usize] as char);
    }
    out
}

fn query_path_sizes(paths: &[&str]) -> HashMap<String, u64> {
    let mut args: Vec<&str> = vec![
        "path-info",
        "--json",
        "--extra-experimental-features",
        "nix-command",
    ];
    args.extend(paths);
    let Some(raw) = run_nix(&args, PATH_INFO_TIMEOUT) else {
        return HashMap::new();
    };
    serde_json::from_slice::<serde_json::Value>(&raw)
        .map(|json| parse_path_info(&json))
        .unwrap_or_default()
}

/// Parse `nix path-info --json` output into a `store path -> narSize` map.
/// Supports both of these versions:
/// - object keyed by store path: (`{"/nix/store/a": {...}}`).
/// - array of objects: (`[{"path": "/nix/store/a", "narSize": 1}, ...]`).
fn parse_path_info(json: &serde_json::Value) -> HashMap<String, u64> {
    let mut out = HashMap::new();
    if let Some(entries) = json.as_array() {
        for entry in entries {
            if let (Some(path), Some(size)) = (
                entry.get("path").and_then(|v| v.as_str()),
                entry.get("narSize").and_then(|v| v.as_u64()),
            ) {
                out.insert(path.to_string(), size);
            }
        }
    } else if let Some(entries) = json.as_object() {
        for (path, entry) in entries {
            if let Some(size) = entry.get("narSize").and_then(|v| v.as_u64()) {
                out.insert(path.clone(), size);
            }
        }
    }
    out
}

fn run_nix(args: &[&str], timeout: Duration) -> Option<Vec<u8>> {
    use std::io::Read;

    let mut child = Command::new("nix")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;

    let mut stdout_pipe = child.stdout.take()?;
    let stdout_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout_pipe.read_to_end(&mut buf);
        buf
    });
    let mut stderr_pipe = child.stderr.take()?;
    let stderr_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr_pipe.read_to_end(&mut buf);
        buf
    });

    let start = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    tracing::debug!("nix {} timed out after {:?}", args.join(" "), timeout);
                    return None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };

    let stdout = stdout_reader.join().unwrap_or_default();
    if status.success() {
        Some(stdout)
    } else {
        let stderr = stderr_reader.join().unwrap_or_default();
        tracing::debug!(
            "nix {} failed ({status}): {}",
            args.join(" "),
            String::from_utf8_lossy(&stderr).trim(),
        );
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_path_from_nar_hash_matches_nix() {
        assert_eq!(
            store_path_from_nar_hash(
                "/nix/store",
                "sha256-x5UQuRsH3MqI0U9afaXSNqzTPSeZlRLvFAav2Ux1pNw=",
            )
            .as_deref(),
            Some("/nix/store/3a2vdn5i7vd2wl654xs8nb52jf1v6cbh-source"),
        );
    }

    #[test]
    fn store_path_from_nar_hash_rejects_non_sri_sha256() {
        for hash in [
            "",
            "not-a-hash",
            "sha512-x5UQuRsH3MqI0U9afaXSNqzTPSeZlRLvFAav2Ux1pNw=",
            "sha256-@@@invalid@@@",
            // Valid base64 but not 32 bytes of digest.
            "sha256-YWJj",
        ] {
            assert_eq!(
                store_path_from_nar_hash("/nix/store", hash),
                None,
                "hash {hash:?} must not produce a store path",
            );
        }
    }

    #[test]
    fn parse_path_info_handles_array_shape() {
        let json: serde_json::Value = serde_json::from_str(
            r#"[
                {"path": "/nix/store/a", "narSize": 100},
                {"path": "/nix/store/b", "narSize": 200}
            ]"#,
        )
        .unwrap();
        let map = parse_path_info(&json);
        assert_eq!(map.get("/nix/store/a"), Some(&100));
        assert_eq!(map.get("/nix/store/b"), Some(&200));
    }

    #[test]
    fn parse_path_info_handles_object_shape() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{
                "/nix/store/a": {"narSize": 100},
                "/nix/store/b": {"narSize": 200}
            }"#,
        )
        .unwrap();
        let map = parse_path_info(&json);
        assert_eq!(map.get("/nix/store/a"), Some(&100));
        assert_eq!(map.get("/nix/store/b"), Some(&200));
    }

    #[test]
    fn query_maps_follows_chain_to_shared_store_path() {
        let lock = FlakeLock::read_from_str(
            r#"{
  "nodes": {
    "nixpkgs": {
      "locked": {
        "narHash": "sha256-x5UQuRsH3MqI0U9afaXSNqzTPSeZlRLvFAav2Ux1pNw=",
        "owner": "nixos", "repo": "nixpkgs", "rev": "aaa", "type": "github"
      },
      "original": { "owner": "nixos", "repo": "nixpkgs", "type": "github" }
    },
    "home-manager": {
      "inputs": { "nixpkgs": ["nixpkgs"] },
      "locked": {
        "narHash": "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        "owner": "o", "repo": "r", "rev": "bbb", "type": "github"
      },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "root": {
      "inputs": { "nixpkgs": "nixpkgs", "home-manager": "home-manager" }
    }
  },
  "root": "root",
  "version": 7
}"#,
        )
        .expect("lock parses");
        let nested = lock.nested_inputs();

        let nixpkgs_path = "/nix/store/3a2vdn5i7vd2wl654xs8nb52jf1v6cbh-source";
        let sizes = InputSizes::query_with(&lock, &nested, |paths| {
            assert!(
                paths.contains(&nixpkgs_path),
                "computed store path must reach the size lookup, got {paths:?}",
            );
            paths.iter().map(|p| ((*p).to_string(), 42)).collect()
        });

        let top: AttrPath = "nixpkgs".parse().unwrap();
        let follower: AttrPath = "home-manager.nixpkgs".parse().unwrap();
        assert_eq!(sizes.store_path_for(&top), Some(nixpkgs_path));
        assert_eq!(
            sizes.store_path_for(&follower),
            sizes.store_path_for(&top),
            "follows chain must land on the target's store path",
        );
        assert_eq!(sizes.size_for(&follower), Some(42));
        assert_eq!(
            sizes.label_for_store_path(nixpkgs_path),
            Some("nixpkgs"),
            "shortest attribute path must label the shared store path",
        );
    }

    #[test]
    fn from_attr_entries_shares_sizes_via_store_path() {
        let sizes = InputSizes::from_attr_entries([
            ("nixpkgs".to_string(), "/store/a".to_string(), Some(7)),
            ("crane.nixpkgs".to_string(), "/store/a".to_string(), None),
            ("crane".to_string(), "/store/b".to_string(), None),
        ]);
        let follower: AttrPath = "crane.nixpkgs".parse().unwrap();
        let unknown: AttrPath = "crane".parse().unwrap();
        assert_eq!(sizes.size_for(&follower), Some(7));
        assert_eq!(sizes.size_for(&unknown), None);
        assert_eq!(sizes.label_for_store_path("/store/a"), Some("nixpkgs"));
    }
}
