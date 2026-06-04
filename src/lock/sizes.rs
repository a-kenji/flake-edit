//! Disk-size lookups for flake inputs.
//!
//! `flake-edit follow --stats` needs the NAR size of every input
//! reachable from the user's flake. We do this with two Nix calls that
//! operate directly on the lockfile:
//!
//! 1. `nix eval --json` of an expression that reads `flake.lock`,
//!    computes the deterministic store path of each node from its
//!    `narHash`, and returns a map of `node_key -> store_path`. The
//!    path is pure arithmetic (no fetch, no verification), so a
//!    force-pushed-away upstream rev cannot zero out the report.
//! 2. `nix path-info --json` on the unique store paths that exist on
//!    disk, returning `narSize` for each.
//!
//! The result is a map keyed by dotted attribute path so the caller
//! looks up sizes by the same path it gets from
//! [`crate::lock::FlakeLock::nested_inputs`] without any URL
//! construction. Multi-segment paths resolve through follows chains to
//! the terminal node, so `home-manager.nixpkgs` and the top-level
//! `nixpkgs` map to the same store path and report the same size.
//!
//! Failures are non-fatal everywhere: a missing `nix`, an eval error,
//! a slow store all manifest as `None` so the caller can render "size
//! unknown" instead of aborting.
//!
//! ## Test injection
//!
//! When [`FIXTURE_ENV`] is set, [`InputSizes::for_flake`] reads sizes
//! from the named JSON file instead of invoking `nix`. The fixture
//! maps dotted attribute paths to byte counts:
//!
//! ```json
//! {
//!   "nixpkgs": 200000000,
//!   "home-manager.nixpkgs": 200000000
//! }
//! ```

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::follows::AttrPath;
use crate::lock::FlakeLock;

/// Environment variable used by tests to inject sizes without invoking
/// `nix`.
pub const FIXTURE_ENV: &str = "FE_FOLLOW_SIZE_FIXTURE";

/// Wall-clock cap for the single `nix eval` invocation. The expression
/// is pure path arithmetic over every locked node, but a flake with
/// hundreds of inputs still has per-node overhead. 60 s covers it.
const EVAL_TIMEOUT: Duration = Duration::from_secs(60);

/// Wall-clock cap for `nix path-info`. Local store stat is fast even at
/// scale; 30 s is already a hard error.
const PATH_INFO_TIMEOUT: Duration = Duration::from_secs(30);

/// Sizes keyed by dotted attribute path, plus the store path each
/// attribute resolves to. The store-path side is what lets callers
/// group followers by their actual shared store entry: many attribute
/// paths that follow the same nixpkgs collapse to one store path and
/// should appear in the summary as one row.
#[derive(Debug, Default)]
pub struct InputSizes {
    by_path: HashMap<String, u64>,
    /// Attribute path to resolved store path. Same key set as
    /// [`Self::by_path`] in the common case, but kept separate so the
    /// absence of a size doesn't lose the grouping information.
    store_paths: HashMap<String, String>,
}

impl InputSizes {
    /// An empty size map. Equivalent to `for_flake` returning `None`
    /// from a caller's perspective; lets the renderer always operate on
    /// a real reference.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Build a size map for the flake at `flake_path` (a path to either
    /// `flake.nix` or the directory containing it).
    ///
    /// Returns `None` if the lockfile is missing or `nix` is not
    /// available. Per-entry failures degrade to a missing key in the
    /// map, not a hard error.
    pub fn for_flake(flake_path: &Path) -> Option<Self> {
        if let Some((by_path, store_paths)) = load_fixture() {
            return Some(Self {
                by_path,
                store_paths,
            });
        }

        let flake_dir = flake_dir_of(flake_path)?;
        let lock_path = flake_dir.join("flake.lock");
        let lock = FlakeLock::from_file(&lock_path).ok()?;

        // Map every attribute path we care about (top-level inputs plus
        // all nested inputs) to the terminal node key in the lock,
        // chasing `follows` chains. Two attrpaths that follow each other
        // land on the same node key.
        let attr_to_node = build_attr_to_node_map(&lock);
        if attr_to_node.is_empty() {
            return Some(Self::empty());
        }

        let node_paths = query_node_paths(&lock_path)?;
        let sizes_by_store = query_path_sizes(&node_paths)?;

        let mut by_path = HashMap::new();
        let mut store_paths = HashMap::new();
        for (attr, node_key) in attr_to_node {
            let Some(store_path) = node_paths.get(&node_key) else {
                continue;
            };
            store_paths.insert(attr.clone(), store_path.clone());
            if let Some(size) = sizes_by_store.get(store_path) {
                by_path.insert(attr, *size);
            }
        }

        Some(Self {
            by_path,
            store_paths,
        })
    }

    /// Look up the size for an attribute path. Returns `None` when the
    /// path is unknown or the size couldn't be determined.
    pub fn for_attr_path(&self, path: &AttrPath) -> Option<u64> {
        self.by_path.get(&dotted(path)).copied()
    }

    /// Resolve an attribute path to the store path it shares with every
    /// other attribute path that follows the same terminal node.
    /// Returns `None` when no resolution is known.
    pub fn store_path_for_attr_path(&self, path: &AttrPath) -> Option<&str> {
        self.store_paths.get(&dotted(path)).map(String::as_str)
    }

    /// Return the shortest attribute path that maps to `store_path`, or
    /// `None` if no path is known to map there. Used as a display label
    /// when collapsing many followers onto one store-path row: the
    /// shortest attribute is "more canonical" (typically the top-level
    /// input the others follow into).
    pub fn shortest_attr_for_store_path(&self, store_path: &str) -> Option<&str> {
        self.store_paths
            .iter()
            .filter(|(_, sp)| sp.as_str() == store_path)
            .map(|(attr, _)| attr.as_str())
            .min_by_key(|attr| (attr.matches('.').count(), attr.len()))
    }
}

/// Render an [`AttrPath`] as a dotted key, using the raw segment
/// strings (not the quoted display form). Matches the key shape we use
/// for indexing across the module.
fn dotted(path: &AttrPath) -> String {
    path.segments()
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join(".")
}

/// Resolve the directory holding `flake.nix`, accepting either a file
/// path or a directory path. A non-existent path is treated as a
/// directory; Nix will produce an error and we'll degrade.
fn flake_dir_of(path: &Path) -> Option<std::path::PathBuf> {
    if path.is_dir() {
        Some(path.to_path_buf())
    } else if path.is_file() {
        path.parent().map(Path::to_path_buf)
    } else {
        Some(path.to_path_buf())
    }
}

/// Load the test fixture from [`FIXTURE_ENV`] if set. Two shapes:
///
/// - `{"attr": 12345}`: size only. The attribute path doubles as its
///   own synthetic store path, so each attribute lands in its own group
///   in the renderer.
/// - `{"attr": {"size": 12345, "store_path": "/some/store/path"}}`:
///   explicit both. Multiple attributes sharing a `store_path` collapse
///   into one group, mirroring what real Nix returns for followers that
///   route to the same terminal node.
fn load_fixture() -> Option<(HashMap<String, u64>, HashMap<String, String>)> {
    let path = std::env::var_os(FIXTURE_ENV)?;
    let raw = std::fs::read_to_string(&path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let obj = value.as_object()?;
    let mut sizes = HashMap::new();
    let mut store_paths = HashMap::new();
    for (k, v) in obj {
        if let Some(n) = v.as_u64() {
            sizes.insert(k.clone(), n);
            store_paths.insert(k.clone(), format!("fixture://{k}"));
        } else if let Some(inner) = v.as_object() {
            if let Some(n) = inner.get("size").and_then(|v| v.as_u64()) {
                sizes.insert(k.clone(), n);
            }
            let store_path = inner
                .get("store_path")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| format!("fixture://{k}"));
            store_paths.insert(k.clone(), store_path);
        }
    }
    Some((sizes, store_paths))
}

/// Build a map from every "interesting" attribute path (top-level
/// inputs and all nested inputs) to the terminal node key in the lock.
/// Paths whose follows chain is nulled or otherwise broken are silently
/// omitted.
fn build_attr_to_node_map(lock: &FlakeLock) -> HashMap<String, String> {
    let mut out: HashMap<String, String> = HashMap::new();
    for (name, key) in lock.top_level_input_node_keys() {
        out.insert(name, key);
    }
    for nested in lock.nested_inputs() {
        if let Some(key) = lock.node_key_for(&nested.path) {
            out.insert(dotted(&nested.path), key);
        }
    }
    out
}

/// Evaluate a Nix expression that computes the deterministic store path
/// of each locked node from its `narHash`, without fetching.
///
/// `builtins.derivation` with the lock's fixed-output hash spec
/// (`sha256`, recursive, the node's `narHash`) produces the identical
/// store path Nix's makeFixedOutputPath would, as pure path arithmetic:
/// no network, no verification, no failure mode that depends on the
/// upstream still hosting that rev. The path is materialized via the
/// `.outPath` attribute; the derivation itself is never built.
///
/// The root node has no `locked` block and shows up as `null` in the
/// JSON, which we then drop.
fn query_node_paths(lock_path: &Path) -> Option<HashMap<String, String>> {
    let lock_str = lock_path.to_string_lossy();
    // JSON encoding produces a valid Nix string literal: same quoting
    // conventions, same escape rules. Avoids hand-rolled escaping.
    let lock_literal = serde_json::to_string(lock_str.as_ref()).ok()?;
    let expr = format!(
        r#"
let
  lock = builtins.fromJSON (builtins.readFile {lock_literal});
  computePath = node:
    if node ? locked && node.locked ? narHash
    then
      (builtins.derivation {{
        name = "source";
        system = builtins.currentSystem;
        builder = "no-builder";
        outputHashAlgo = "sha256";
        outputHashMode = "recursive";
        outputHash = node.locked.narHash;
      }}).outPath
    else null;
in
  builtins.mapAttrs (_: node: computePath node) lock.nodes
"#
    );

    // `--extra-experimental-features` makes sure `nix-command` is on
    // even if the user hasn't enabled it globally. `--impure` is
    // required by `builtins.currentSystem`.
    let raw = run_nix(
        &[
            "eval",
            "--json",
            "--impure",
            "--extra-experimental-features",
            "nix-command",
            "--expr",
            &expr,
        ],
        None,
        EVAL_TIMEOUT,
    )?;
    let json: serde_json::Value = serde_json::from_slice(&raw).ok()?;
    Some(
        json.as_object()?
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
            .collect(),
    )
}

/// Query NAR sizes for the unique store paths in `node_paths`.
///
/// `nix path-info` fails the whole batch if any single path isn't in
/// the store. Filter to paths that actually exist on disk before asking
/// Nix so one missing input doesn't blank every size.
fn query_path_sizes(node_paths: &HashMap<String, String>) -> Option<HashMap<String, u64>> {
    let unique: HashSet<&str> = node_paths
        .values()
        .map(String::as_str)
        .filter(|p| Path::new(p).exists())
        .collect();
    if unique.is_empty() {
        return Some(HashMap::new());
    }

    let mut args: Vec<&str> = vec![
        "path-info",
        "--json",
        "--extra-experimental-features",
        "nix-command",
    ];
    args.extend(unique);
    let raw = run_nix(&args, None, PATH_INFO_TIMEOUT)?;
    let json: serde_json::Value = serde_json::from_slice(&raw).ok()?;
    Some(parse_path_info(&json))
}

/// Parse `nix path-info --json` output into a `store_path -> size` map.
/// Nix has emitted two shapes historically; handle both:
///
/// - Old: object keyed by store path (`{ "/nix/store/a": { ... } }`).
/// - New: array of objects with a `path` field
///   (`[{"path": "/nix/store/a", "narSize": ...}, ...]`).
fn parse_path_info(json: &serde_json::Value) -> HashMap<String, u64> {
    let mut out = HashMap::new();
    if let Some(arr) = json.as_array() {
        for entry in arr {
            if let (Some(path), Some(size)) = (
                entry.get("path").and_then(|v| v.as_str()),
                entry.get("narSize").and_then(|v| v.as_u64()),
            ) {
                out.insert(path.to_string(), size);
            }
        }
    } else if let Some(obj) = json.as_object() {
        for (path, entry) in obj {
            if let Some(size) = entry.get("narSize").and_then(|v| v.as_u64()) {
                out.insert(path.clone(), size);
            }
        }
    }
    out
}

/// Run `nix <args>` with a wall-clock timeout. `cwd` sets the
/// subprocess's working directory when present; otherwise the parent's
/// CWD is inherited.
///
/// Captures stderr and emits it via [`tracing::debug`] when the
/// invocation fails or times out, so `FE_LOG=debug` users can see what
/// `nix` is complaining about without spamming a successful run.
fn run_nix(args: &[&str], cwd: Option<&Path>, timeout: Duration) -> Option<Vec<u8>> {
    let mut cmd = Command::new("nix");
    cmd.args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    let mut child = cmd.spawn().ok()?;

    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let output = child.wait_with_output().ok()?;
                if !status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    tracing::debug!(
                        "nix {} failed ({}): {}",
                        args.join(" "),
                        status,
                        stderr.trim()
                    );
                    return None;
                }
                return Some(output.stdout);
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    tracing::debug!(
                        "nix {} timed out after {}s",
                        args.join(" "),
                        timeout.as_secs()
                    );
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
