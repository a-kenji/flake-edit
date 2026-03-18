use crate::error::FlakeEditError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

/// A nested input path with optional existing follows target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NestedInput {
    /// The path to the nested input (e.g., "crane.nixpkgs")
    pub path: String,
    /// The target this input follows, if any (e.g., "nixpkgs")
    pub follows: Option<String>,
    /// The original flake URL for Direct references (e.g., "github:nixos/nixpkgs/nixos-unstable")
    pub url: Option<String>,
}

impl NestedInput {
    /// Format for display: "path\tfollows_target" or just "path".
    /// The tab separator allows the UI to parse and style the parts differently.
    pub fn to_display_string(&self) -> String {
        match &self.follows {
            Some(target) => format!("{}\t{}", self.path, target),
            None => self.path.clone(),
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
    /// Split an input path into segments, respecting quoted names.
    /// E.g. `"hls-1.10".nixpkgs` -> `["hls-1.10", "nixpkgs"]`
    /// E.g. `browseros.nixpkgs` -> `["browseros", "nixpkgs"]`
    fn split_input_path(path: &str) -> Vec<&str> {
        let mut segments = Vec::new();
        let mut rest = path;
        while !rest.is_empty() {
            if rest.starts_with('"') {
                // Quoted segment: find closing quote
                if let Some(end) = rest[1..].find('"') {
                    segments.push(&rest[1..end + 1]);
                    rest = &rest[end + 2..];
                    // Skip the dot separator if present
                    rest = rest.strip_prefix('.').unwrap_or(rest);
                } else {
                    // Malformed: no closing quote, treat rest as one segment
                    segments.push(rest.trim_matches('"'));
                    break;
                }
            } else if let Some(dot) = rest.find('.') {
                segments.push(&rest[..dot]);
                rest = &rest[dot + 1..];
            } else {
                segments.push(rest);
                break;
            }
        }
        segments
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
        let segments = Self::split_input_path(id);
        let node_name = self.resolve_input_path(&segments)?;
        let node = self.nodes.get(&node_name).ok_or_else(|| {
            FlakeEditError::LockError(format!("Could not find node '{node_name}'."))
        })?;
        node.rev()
    }

    /// Get all nested input paths for shell completions.
    /// Returns paths like "naersk.nixpkgs", "naersk.flake-utils", etc.
    pub fn nested_input_paths(&self) -> Vec<String> {
        self.nested_inputs()
            .into_iter()
            .map(|input| input.path)
            .collect()
    }

    /// Get all nested inputs with their existing follows targets.
    pub fn nested_inputs(&self) -> Vec<NestedInput> {
        let mut inputs = Vec::new();

        // Get the root node
        let Some(root_node) = self.nodes.get(&self.root) else {
            return inputs;
        };

        // Get top-level inputs from root
        let Some(root_inputs) = &root_node.inputs else {
            return inputs;
        };

        // For each top-level input, find its nested inputs
        for (top_level_name, top_level_ref) in root_inputs {
            // Resolve the node name (could be different from input name)
            let node_name = match top_level_ref {
                Input::Direct(name) => name.clone(),
                Input::Indirect(_) => {
                    // For indirect inputs (follows), skip - they don't have their own inputs
                    continue;
                }
            };

            // Get the node for this input
            if let Some(node) = self.nodes.get(&node_name) {
                // Get nested inputs of this node
                if let Some(nested_inputs) = &node.inputs {
                    for (nested_name, nested_ref) in nested_inputs {
                        let quoted_parent = if top_level_name.contains('.') {
                            format!("\"{}\"", top_level_name)
                        } else {
                            top_level_name.clone()
                        };
                        let quoted_nested = if nested_name.contains('.') {
                            format!("\"{}\"", nested_name)
                        } else {
                            nested_name.clone()
                        };
                        let path = format!("{}.{}", quoted_parent, quoted_nested);
                        let (follows, url) = match nested_ref {
                            Input::Indirect(targets) => (Some(targets.join(".")), None),
                            Input::Direct(node_name) => {
                                let url = self
                                    .nodes
                                    .get(node_name.as_str())
                                    .and_then(|n| n.original.as_ref())
                                    .and_then(|o| o.to_flake_url());
                                (None, url)
                            }
                        };
                        inputs.push(NestedInput { path, follows, url });
                    }
                }
            }
        }

        inputs.sort_by(|a, b| a.path.cmp(&b.path));
        inputs
    }
}
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
        assert_eq!(nested[0].path, "\"hls-1.10\".nixpkgs");
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

    #[test]
    fn split_input_path_simple() {
        assert_eq!(FlakeLock::split_input_path("nixpkgs"), vec!["nixpkgs"]);
    }

    #[test]
    fn split_input_path_dotted() {
        assert_eq!(
            FlakeLock::split_input_path("treefmt-nix.nixpkgs"),
            vec!["treefmt-nix", "nixpkgs"]
        );
    }

    #[test]
    fn split_input_path_quoted() {
        assert_eq!(
            FlakeLock::split_input_path("\"hls-1.10\".nixpkgs"),
            vec!["hls-1.10", "nixpkgs"]
        );
    }
}
