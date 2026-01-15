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
    fn get_rev(&self) -> String {
        self.locked.clone().unwrap().get_rev().to_string()
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum Input {
    Direct(String),
    Indirect(Vec<String>),
}

impl Input {
    fn get_id(&self) -> String {
        match self {
            Input::Direct(id) => id.to_string(),
            Input::Indirect(_) => todo!(),
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
    fn get_rev(&self) -> String {
        self.rev.clone().unwrap()
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
    /// Query the lock file for a specific rev.
    pub fn get_rev_by_id(&self, id: &str) -> Result<String, FlakeEditError> {
        let root = self.root();
        let resolved_root = self
            .nodes
            .get(root)
            .ok_or(FlakeEditError::LockMissingRoot)?;
        let binding = resolved_root
            .inputs
            .clone()
            .ok_or_else(|| FlakeEditError::LockError("Could not resolve root.".into()))?;
        let resolved_id = binding
            .get(id)
            .ok_or_else(|| FlakeEditError::LockError("Could not resolve id.".into()))?;
        let id = resolved_id.get_id();
        let node = self
            .nodes
            .get(&id)
            .ok_or_else(|| FlakeEditError::LockError("Could not find node with id.".into()))?;
        Ok(node.get_rev())
    }

    /// Get all nested input paths for shell completions.
    /// Returns paths like "naersk.nixpkgs", "naersk.flake-utils", etc.
    pub fn get_nested_input_paths(&self) -> Vec<String> {
        self.get_nested_inputs()
            .into_iter()
            .map(|input| input.path)
            .collect()
    }

    /// Get all nested inputs with their existing follows targets.
    pub fn get_nested_inputs(&self) -> Vec<NestedInput> {
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
                        let path = format!("{}.{}", top_level_name, nested_name);
                        let follows = match nested_ref {
                            Input::Indirect(targets) => Some(targets.join(".")),
                            Input::Direct(_) => None,
                        };
                        inputs.push(NestedInput { path, follows });
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
                .get_rev_by_id("nixpkgs")
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
                .get_rev_by_id("nixpkgs")
                .expect("Id: nixpkgs is in the lockfile.")
        );
    }
    #[test]
    fn parse_minimal_independent_lock_nixpkgs_overridden() {
        let minimal_lock = minimal_independent_lock_nixpkgs_overridden();
        FlakeLock::read_from_str(minimal_lock).expect("Should be parsed correctly.");
    }
}
