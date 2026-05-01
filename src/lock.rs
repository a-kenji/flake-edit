use crate::error::FlakeEditError;
use crate::follows::{AttrPath, Segment};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

/// A nested input path with optional existing follows target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NestedInput {
    /// The path to the nested input (e.g., `crane.nixpkgs`).
    pub path: AttrPath,
    /// The target this input follows, if any.
    pub follows: Option<AttrPath>,
    /// The original flake URL for Direct references
    /// (e.g., `github:nixos/nixpkgs/nixos-unstable`).
    pub url: Option<String>,
}

impl NestedInput {
    /// Format for display: `path\tfollows_target` or just `path`.
    /// The tab separator allows the UI to parse and style the parts differently.
    pub fn to_display_string(&self) -> String {
        match &self.follows {
            Some(target) => format!("{}\t{}", self.path, target),
            None => self.path.to_string(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FlakeLock {
    nodes: HashMap<String, Node>,
    root: String,
    version: u8,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Node {
    inputs: Option<HashMap<String, Input>>,
    locked: Option<Locked>,
    original: Option<Original>,
}

impl Node {
    fn rev(&self) -> Result<String, FlakeEditError> {
        self.locked
            .as_ref()
            .ok_or_else(|| FlakeEditError::LockError("Node has no locked information.".into()))?
            .rev()
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum Input {
    Direct(String),
    Indirect(Vec<String>),
}

impl Input {
    /// Get the target node name for this input.
    /// For Direct inputs, returns the node name directly.
    /// For Indirect inputs (follows paths), returns the final target in the path.
    fn id(&self) -> String {
        match self {
            Input::Direct(id) => id.to_string(),
            Input::Indirect(path) => path.last().cloned().unwrap_or_default(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Locked {
    owner: Option<String>,
    repo: Option<String>,
    rev: Option<String>,
    #[serde(rename = "type")]
    node_type: String,
    #[serde(rename = "ref")]
    ref_field: Option<String>,
}

impl Locked {
    fn rev(&self) -> Result<String, FlakeEditError> {
        self.rev
            .clone()
            .ok_or_else(|| FlakeEditError::LockError("Locked node has no rev.".into()))
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Original {
    owner: Option<String>,
    repo: Option<String>,
    #[serde(rename = "type")]
    node_type: String,
    #[serde(rename = "ref")]
    ref_field: Option<String>,
    url: Option<String>,
}

impl Original {
    /// Reconstruct a flake URL from the original reference.
    pub fn to_flake_url(&self) -> Option<String> {
        match self.node_type.as_str() {
            "github" | "gitlab" | "sourcehut" => {
                let owner = self.owner.as_deref()?;
                let repo = self.repo.as_deref()?;
                let mut url = format!("{}:{}/{}", self.node_type, owner, repo);
                if let Some(ref_field) = &self.ref_field {
                    url.push('/');
                    url.push_str(ref_field);
                }
                Some(url)
            }
            _ => self.url.clone(),
        }
    }
}

impl FlakeLock {
    const LOCK: &'static str = "flake.lock";

    pub fn from_default_path() -> Result<Self, FlakeEditError> {
        let path = PathBuf::from(Self::LOCK);
        Self::from_file(path)
    }

    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, FlakeEditError> {
        let mut file = File::open(path)?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        Self::read_from_str(&contents)
    }
    pub fn read_from_str(str: &str) -> Result<Self, FlakeEditError> {
        Ok(serde_json::from_str(str)?)
    }
    pub fn root(&self) -> &str {
        &self.root
    }
    /// Resolve an input path to a node name by walking the lock tree.
    fn resolve_input_path(&self, segments: &[&str]) -> Result<String, FlakeEditError> {
        let mut current_node = self
            .nodes
            .get(self.root())
            .ok_or(FlakeEditError::LockMissingRoot)?;

        for (i, segment) in segments.iter().enumerate() {
            let inputs = current_node.inputs.as_ref().ok_or_else(|| {
                if i == 0 {
                    FlakeEditError::LockError("Could not resolve root.".into())
                } else {
                    FlakeEditError::LockError(format!(
                        "Input '{}' has no sub-inputs.",
                        segments[..i].join(".")
                    ))
                }
            })?;

            let resolved = inputs.get(*segment).ok_or_else(|| {
                FlakeEditError::LockError(format!(
                    "Input '{}' not found in lock file.",
                    segments[..=i].join(".")
                ))
            })?;

            let node_name = resolved.id();

            if i < segments.len() - 1 {
                // Intermediate segment: move to the next node
                current_node = self.nodes.get(&node_name).ok_or_else(|| {
                    FlakeEditError::LockError(format!(
                        "Could not find node '{}' for input '{}'.",
                        node_name,
                        segments[..=i].join(".")
                    ))
                })?;
            } else {
                // Final segment: return the node name
                return Ok(node_name);
            }
        }

        Err(FlakeEditError::LockError("Empty input path.".into()))
    }

    /// Query the lock file for a specific rev.
    pub fn rev_for(&self, id: &str) -> Result<String, FlakeEditError> {
        let path = AttrPath::parse(id)
            .map_err(|e| FlakeEditError::LockError(format!("Invalid input path '{id}': {e}")))?;
        let owned: Vec<&str> = path.segments().iter().map(|s| s.as_str()).collect();
        let node_name = self.resolve_input_path(&owned)?;
        let node = self.nodes.get(&node_name).ok_or_else(|| {
            FlakeEditError::LockError(format!("Could not find node '{node_name}'."))
        })?;
        node.rev()
    }

    /// Get all nested input paths for shell completions.
    /// Returns rendered paths like `naersk.nixpkgs`, `naersk.flake-utils`.
    pub fn nested_input_paths(&self) -> Vec<String> {
        self.nested_inputs()
            .into_iter()
            .map(|input| input.path.to_string())
            .collect()
    }

    /// Get all nested inputs with their existing follows targets.
    ///
    /// Walks `flake.lock` recursively from the root. Emits one entry per
    /// `inputs.X` declared on any descendant node, with the path built
    /// segment-by-segment as the walk descends. Bounded by the
    /// [`NESTED_INPUTS_MAX_DEPTH`] constant; cycles in the lockfile node
    /// graph are short-circuited via a visited set keyed on node name.
    ///
    /// The output is sorted by path for stable emission order.
    pub fn nested_inputs(&self) -> Vec<NestedInput> {
        let mut inputs = Vec::new();
        let Some(root_node) = self.nodes.get(&self.root) else {
            return inputs;
        };
        let Some(root_inputs) = &root_node.inputs else {
            return inputs;
        };

        for (top_level_name, top_level_ref) in root_inputs {
            let node_name = match top_level_ref {
                Input::Direct(name) => name.clone(),
                // Indirect (follows) top-level inputs have no sub-inputs to
                // descend through here; resolved-side follows still appear
                // as edges via the recursive walker on the `Direct` siblings
                // that own them.
                Input::Indirect(_) => continue,
            };
            let Ok(parent_seg) = Segment::from_unquoted(top_level_name.clone()) else {
                continue;
            };
            let path = AttrPath::new(parent_seg);
            let mut visited: HashMap<String, ()> = HashMap::new();
            visited.insert(node_name.clone(), ());
            self.collect_nested_inputs_recursive(&node_name, &path, 1, &mut visited, &mut inputs);
        }

        inputs.sort_by(|a, b| a.path.cmp(&b.path));
        inputs
    }

    /// Recursive helper for [`Self::nested_inputs`]. Descends through the
    /// node identified by `node_name`, emitting one [`NestedInput`] per
    /// declared sub-input at any depth up to [`NESTED_INPUTS_MAX_DEPTH`].
    fn collect_nested_inputs_recursive(
        &self,
        node_name: &str,
        parent_path: &AttrPath,
        depth: usize,
        visited: &mut HashMap<String, ()>,
        out: &mut Vec<NestedInput>,
    ) {
        if depth >= NESTED_INPUTS_MAX_DEPTH {
            return;
        }
        let Some(node) = self.nodes.get(node_name) else {
            return;
        };
        let Some(node_inputs) = &node.inputs else {
            return;
        };

        // Iterate sub-inputs in lex order so emission is deterministic.
        let mut keys: Vec<&String> = node_inputs.keys().collect();
        keys.sort();
        for nested_name in keys {
            let nested_ref = node_inputs.get(nested_name).unwrap();
            let Ok(nested_seg) = Segment::from_unquoted(nested_name.clone()) else {
                continue;
            };
            let mut path = parent_path.clone();
            path.push(nested_seg);

            let (follows, url, descend_into) = match nested_ref {
                Input::Indirect(targets) => {
                    // The lockfile's `inputs.X = [a, b, ...]` shape encodes a
                    // resolved follows target as an array of segment names.
                    let mut iter = targets.iter();
                    let first = iter
                        .next()
                        .and_then(|s| Segment::from_unquoted(s.clone()).ok());
                    match first {
                        Some(first_seg) => {
                            let mut follows_path = AttrPath::new(first_seg);
                            let mut all_ok = true;
                            for raw in iter {
                                match Segment::from_unquoted(raw.clone()) {
                                    Ok(seg) => follows_path.push(seg),
                                    Err(_) => {
                                        all_ok = false;
                                        break;
                                    }
                                }
                            }
                            if all_ok {
                                (Some(follows_path), None, None)
                            } else {
                                (None, None, None)
                            }
                        }
                        None => (None, None, None),
                    }
                }
                Input::Direct(child_node_name) => {
                    let url = self
                        .nodes
                        .get(child_node_name.as_str())
                        .and_then(|n| n.original.as_ref())
                        .and_then(|o| o.to_flake_url());
                    (None, url, Some(child_node_name.clone()))
                }
            };

            out.push(NestedInput {
                path: path.clone(),
                follows,
                url,
            });

            if let Some(child) = descend_into {
                if visited.contains_key(&child) {
                    continue;
                }
                visited.insert(child.clone(), ());
                self.collect_nested_inputs_recursive(&child, &path, depth + 1, visited, out);
                visited.remove(&child);
            }
        }
    }
}

/// Default maximum depth for [`FlakeLock::nested_inputs`]. Backstop against
/// cycles in malformed lockfiles.
pub const NESTED_INPUTS_MAX_DEPTH: usize = 64;
#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_lock() -> &'static str {
        r#"
    {
  "nodes": {
    "nixpkgs": {
      "locked": {
        "lastModified": 1718714799,
        "narHash": "sha256-FUZpz9rg3gL8NVPKbqU8ei1VkPLsTIfAJ2fdAf5qjak=",
        "owner": "nixos",
        "repo": "nixpkgs",
        "rev": "c00d587b1a1afbf200b1d8f0b0e4ba9deb1c7f0e",
        "type": "github"
      },
      "original": {
        "owner": "nixos",
        "ref": "nixos-unstable",
        "repo": "nixpkgs",
        "type": "github"
      }
    },
    "root": {
      "inputs": {
        "nixpkgs": "nixpkgs"
      }
    }
  },
  "root": "root",
  "version": 7
}
    "#
    }
    fn minimal_independent_lock_no_overrides() -> &'static str {
        r#"
    {
  "nodes": {
    "nixpkgs": {
      "locked": {
        "lastModified": 1721138476,
        "narHash": "sha256-+W5eZOhhemLQxelojLxETfbFbc19NWawsXBlapYpqIA=",
        "owner": "nixos",
        "repo": "nixpkgs",
        "rev": "ad0b5eed1b6031efaed382844806550c3dcb4206",
        "type": "github"
      },
      "original": {
        "owner": "nixos",
        "ref": "nixos-unstable",
        "repo": "nixpkgs",
        "type": "github"
      }
    },
    "nixpkgs_2": {
      "locked": {
        "lastModified": 1719690277,
        "narHash": "sha256-0xSej1g7eP2kaUF+JQp8jdyNmpmCJKRpO12mKl/36Kc=",
        "owner": "nixos",
        "repo": "nixpkgs",
        "rev": "2741b4b489b55df32afac57bc4bfd220e8bf617e",
        "type": "github"
      },
      "original": {
        "owner": "nixos",
        "ref": "nixos-unstable",
        "repo": "nixpkgs",
        "type": "github"
      }
    },
    "root": {
      "inputs": {
        "nixpkgs": "nixpkgs",
        "treefmt-nix": "treefmt-nix"
      }
    },
    "treefmt-nix": {
      "inputs": {
        "nixpkgs": "nixpkgs_2"
      },
      "locked": {
        "lastModified": 1721382922,
        "narHash": "sha256-GYpibTC0YYKRpFR9aftym9jjRdUk67ejw1IWiaQkaiU=",
        "owner": "numtide",
        "repo": "treefmt-nix",
        "rev": "50104496fb55c9140501ea80d183f3223d13ff65",
        "type": "github"
      },
      "original": {
        "owner": "numtide",
        "repo": "treefmt-nix",
        "type": "github"
      }
    }
  },
  "root": "root",
  "version": 7
}
    "#
    }

    fn minimal_independent_lock_nixpkgs_overridden() -> &'static str {
        r#"
    {
  "nodes": {
    "nixpkgs": {
      "locked": {
        "lastModified": 1721138476,
        "narHash": "sha256-+W5eZOhhemLQxelojLxETfbFbc19NWawsXBlapYpqIA=",
        "owner": "nixos",
        "repo": "nixpkgs",
        "rev": "ad0b5eed1b6031efaed382844806550c3dcb4206",
        "type": "github"
      },
      "original": {
        "owner": "nixos",
        "ref": "nixos-unstable",
        "repo": "nixpkgs",
        "type": "github"
      }
    },
    "root": {
      "inputs": {
        "nixpkgs": "nixpkgs",
        "treefmt-nix": "treefmt-nix"
      }
    },
    "treefmt-nix": {
      "inputs": {
        "nixpkgs": [
          "nixpkgs"
        ]
      },
      "locked": {
        "lastModified": 1721382922,
        "narHash": "sha256-GYpibTC0YYKRpFR9aftym9jjRdUk67ejw1IWiaQkaiU=",
        "owner": "numtide",
        "repo": "treefmt-nix",
        "rev": "50104496fb55c9140501ea80d183f3223d13ff65",
        "type": "github"
      },
      "original": {
        "owner": "numtide",
        "repo": "treefmt-nix",
        "type": "github"
      }
    }
  },
  "root": "root",
  "version": 7
}
    "#
    }

    #[test]
    fn parse_minimal() {
        let minimal_lock = minimal_lock();
        FlakeLock::read_from_str(minimal_lock).expect("Should be parsed correctly.");
    }
    #[test]
    fn parse_minimal_version() {
        let minimal_lock = minimal_lock();
        let parsed_lock =
            FlakeLock::read_from_str(minimal_lock).expect("Should be parsed correctly.");
        assert_eq!(7, parsed_lock.version);
    }
    #[test]
    fn parse_minimal_root() {
        let minimal_lock = minimal_lock();
        let parsed_lock =
            FlakeLock::read_from_str(minimal_lock).expect("Should be parsed correctly.");
        assert_eq!("root", parsed_lock.root);
    }
    #[test]
    fn minimal_ref() {
        let minimal_lock = minimal_lock();
        let parsed_lock =
            FlakeLock::read_from_str(minimal_lock).expect("Should be parsed correctly.");
        assert_eq!(
            "c00d587b1a1afbf200b1d8f0b0e4ba9deb1c7f0e",
            parsed_lock
                .rev_for("nixpkgs")
                .expect("Id: nixpkgs is in the lockfile.")
        );
    }
    #[test]
    fn parse_minimal_independent_lock_no_overrides() {
        let minimal_lock = minimal_independent_lock_no_overrides();
        FlakeLock::read_from_str(minimal_lock).expect("Should be parsed correctly.");
    }
    #[test]
    fn minimal_independent_lock_no_overrides_ref() {
        let minimal_lock = minimal_independent_lock_no_overrides();
        let parsed_lock =
            FlakeLock::read_from_str(minimal_lock).expect("Should be parsed correctly.");
        assert_eq!(
            "ad0b5eed1b6031efaed382844806550c3dcb4206",
            parsed_lock
                .rev_for("nixpkgs")
                .expect("Id: nixpkgs is in the lockfile.")
        );
    }
    #[test]
    fn parse_minimal_independent_lock_nixpkgs_overridden() {
        let minimal_lock = minimal_independent_lock_nixpkgs_overridden();
        FlakeLock::read_from_str(minimal_lock).expect("Should be parsed correctly.");
    }

    #[test]
    fn input_indirect_id() {
        // Follows path like ["nixpkgs"] should return "nixpkgs"
        let input = Input::Indirect(vec!["nixpkgs".to_string()]);
        assert_eq!("nixpkgs", input.id());
    }

    #[test]
    fn rev_for_sub_input_path_missing_parent_returns_error() {
        // Sub-input paths where the parent doesn't exist should error.
        let minimal_lock = minimal_lock();
        let parsed_lock =
            FlakeLock::read_from_str(minimal_lock).expect("Should be parsed correctly.");
        assert!(parsed_lock.rev_for("browseros.nixpkgs").is_err());
    }

    #[test]
    fn rev_for_sub_input_path_resolves() {
        // Sub-input paths like "treefmt-nix.nixpkgs" should traverse the lock tree.
        let lock = minimal_independent_lock_no_overrides();
        let parsed = FlakeLock::read_from_str(lock).expect("Should be parsed correctly.");
        assert_eq!(
            "2741b4b489b55df32afac57bc4bfd220e8bf617e",
            parsed
                .rev_for("treefmt-nix.nixpkgs")
                .expect("Should resolve sub-input path")
        );
    }

    #[test]
    fn rev_for_sub_input_follows_resolves() {
        // Sub-input that follows the root input should resolve to the same rev.
        let lock = minimal_independent_lock_nixpkgs_overridden();
        let parsed = FlakeLock::read_from_str(lock).expect("Should be parsed correctly.");
        assert_eq!(
            parsed.rev_for("nixpkgs").unwrap(),
            parsed
                .rev_for("treefmt-nix.nixpkgs")
                .expect("Should resolve followed sub-input")
        );
    }

    #[test]
    fn rev_for_quoted_id() {
        // Quoted attribute names like `"nixpkgs-24.11"` from `list --format simple`
        // should be stripped before lock lookup.
        let minimal_lock = minimal_lock();
        let parsed_lock =
            FlakeLock::read_from_str(minimal_lock).expect("Should be parsed correctly.");
        assert_eq!(
            parsed_lock.rev_for("nixpkgs").unwrap(),
            parsed_lock.rev_for("\"nixpkgs\"").unwrap(),
        );
    }

    #[test]
    fn rev_for_node_without_locked_returns_error() {
        // A node that exists but has no "locked" field should error, not panic.
        let lock = r#"{
  "nodes": {
    "root": {
      "inputs": { "bare": "bare" }
    },
    "bare": {
      "original": { "owner": "o", "repo": "r", "type": "github" }
    }
  },
  "root": "root",
  "version": 7
}"#;
        let parsed = FlakeLock::read_from_str(lock).unwrap();
        assert!(parsed.rev_for("bare").is_err());
    }

    #[test]
    fn rev_for_node_without_rev_returns_error() {
        // A locked node without a "rev" field should error, not panic.
        let lock = r#"{
  "nodes": {
    "root": {
      "inputs": { "norev": "norev" }
    },
    "norev": {
      "locked": { "lastModified": 1, "narHash": "", "type": "path" },
      "original": { "type": "path" }
    }
  },
  "root": "root",
  "version": 7
}"#;
        let parsed = FlakeLock::read_from_str(lock).unwrap();
        assert!(parsed.rev_for("norev").is_err());
    }

    #[test]
    fn nested_input_path_quotes_dots() {
        // Input names with dots should be quoted in the path
        let lock = r#"{
  "nodes": {
    "hls-1.10": {
      "inputs": { "nixpkgs": "nixpkgs_2" },
      "flake": false,
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "abc", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "nixpkgs": {
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "abc", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "nixpkgs_2": {
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "def", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "root": {
      "inputs": { "hls-1.10": "hls-1.10", "nixpkgs": "nixpkgs" }
    }
  },
  "root": "root",
  "version": 7
}"#;
        let parsed = FlakeLock::read_from_str(lock).unwrap();
        let nested = parsed.nested_inputs();
        assert_eq!(nested.len(), 1);
        assert_eq!(nested[0].path.to_string(), "\"hls-1.10\".nixpkgs");
    }

    #[test]
    fn nested_inputs_recurses_to_grandchild() {
        // The walker descends through `Direct` children, so a depth-2
        // nested input (root → neovim → nixvim → flake-parts) must be
        // emitted with a 3-segment path. This exercises the recursive
        // path-stack code in `collect_nested_inputs_recursive`.
        let lock = r#"{
  "nodes": {
    "flake-parts": {
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "a", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "flake-parts_2": {
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "b", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "neovim": {
      "inputs": { "nixvim": "nixvim" },
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "c", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "nixvim": {
      "inputs": { "flake-parts": "flake-parts_2" },
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "d", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "root": {
      "inputs": { "flake-parts": "flake-parts", "neovim": "neovim" }
    }
  },
  "root": "root",
  "version": 7
}"#;
        let parsed = FlakeLock::read_from_str(lock).unwrap();
        let nested = parsed.nested_inputs();
        let paths: Vec<String> = nested.iter().map(|n| n.path.to_string()).collect();
        assert!(
            paths.contains(&"neovim.nixvim".to_string()),
            "depth-1 path missing, got: {paths:?}"
        );
        assert!(
            paths.contains(&"neovim.nixvim.flake-parts".to_string()),
            "depth-2 path missing, got: {paths:?}"
        );
    }

    #[test]
    fn nested_inputs_terminates_on_cyclic_lockfile() {
        // A pathological lock graph where node A's input recurses back
        // into A itself must not loop forever. The visited-set in
        // `collect_nested_inputs_recursive` short-circuits the cycle.
        let lock = r#"{
  "nodes": {
    "a": {
      "inputs": { "b": "b" },
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "a", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "b": {
      "inputs": { "a": "a" },
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "b", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "root": { "inputs": { "a": "a" } }
  },
  "root": "root",
  "version": 7
}"#;
        let parsed = FlakeLock::read_from_str(lock).unwrap();
        let nested = parsed.nested_inputs();
        assert!(!nested.is_empty());
        assert!(
            nested
                .iter()
                .all(|n| n.path.len() <= NESTED_INPUTS_MAX_DEPTH)
        );
    }

    #[test]
    fn rev_for_quoted_sub_input_path() {
        // Quoted input names with dots like "hls-1.10".nixpkgs should resolve
        let lock = r#"{
  "nodes": {
    "hls-1.10": {
      "inputs": { "nixpkgs": "nixpkgs_2" },
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "abc", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "nixpkgs": {
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "abc", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "nixpkgs_2": {
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "def", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "root": {
      "inputs": { "hls-1.10": "hls-1.10", "nixpkgs": "nixpkgs" }
    }
  },
  "root": "root",
  "version": 7
}"#;
        let parsed = FlakeLock::read_from_str(lock).unwrap();
        assert_eq!(
            "def",
            parsed
                .rev_for("\"hls-1.10\".nixpkgs")
                .expect("Should resolve quoted sub-input path")
        );
    }
}
