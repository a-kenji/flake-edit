use crate::error::FlakeEditError;
use crate::follows::{AttrPath, Segment};
use serde::{Deserialize, Deserializer};
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

/// A nested input discovered in `flake.lock` with its existing follows
/// target, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NestedInput {
    /// Dotted path to the nested input (e.g. `crane.nixpkgs`).
    pub path: AttrPath,
    /// Existing follows target, if the input is redirected.
    pub follows: Option<AttrPath>,
    /// Original flake URL for `Direct` references (e.g.
    /// `github:nixos/nixpkgs/nixos-unstable`). `None` for follows-only
    /// references.
    pub url: Option<String>,
}

impl NestedInput {
    /// Render as `path\tfollows_target` (or just `path` when the input has no
    /// follows target). The tab separator lets the UI style each side
    /// independently.
    pub fn to_display_string(&self) -> String {
        match &self.follows {
            Some(target) => format!("{}\t{}", self.path, target),
            None => self.path.to_string(),
        }
    }
}

/// Parsed `flake.lock`. Loaded with [`Self::from_default_path`],
/// [`Self::from_file`], or [`Self::read_from_str`].
#[derive(Debug, Deserialize)]
pub struct FlakeLock {
    nodes: HashMap<String, Node>,
    root: String,
}

/// A single entry in the lockfile's `nodes` map.
#[derive(Debug, Deserialize)]
pub(crate) struct Node {
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

/// Reference from a node's `inputs` map.
///
/// Lockfile shape:
/// - `"name"` is a direct reference to another node, parsed as
///   [`Self::Direct`].
/// - `["a", "b", ...]` is a follows path with a target, parsed as
///   `Indirect(Some(path))`.
/// - `[]` is a follows declaration with no target (Nix emits this when an
///   upstream chain has overridden the input to nothing, e.g. a lockfile
///   whose source `flake.nix` carries
///   `nix.inputs.flake-compat.follows = "";`), parsed as `Indirect(None)`.
#[derive(Debug, Clone)]
pub enum Input {
    Direct(String),
    Indirect(Option<AttrPath>),
}

impl<'de> Deserialize<'de> for Input {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::{Error, SeqAccess, Visitor};
        use std::fmt;

        struct InputVisitor;

        impl<'de> Visitor<'de> for InputVisitor {
            type Value = Input;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str(
                    "a node name string, an empty array, or an array of \
                     non-empty segment names",
                )
            }

            fn visit_str<E: Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(Input::Direct(v.to_string()))
            }

            fn visit_string<E: Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(Input::Direct(v))
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                // Empty `[]` is a real lockfile shape: `inputs.X = []`
                // marks an input whose follows chain has been overridden
                // away. Surface it as `Indirect(None)` instead of an
                // error.
                let Some(first) = seq.next_element::<String>()? else {
                    return Ok(Input::Indirect(None));
                };
                let first_seg = Segment::from_unquoted(first).map_err(A::Error::custom)?;
                let mut path = AttrPath::new(first_seg);
                while let Some(raw) = seq.next_element::<String>()? {
                    let seg = Segment::from_unquoted(raw).map_err(A::Error::custom)?;
                    path.push(seg);
                }
                Ok(Input::Indirect(Some(path)))
            }
        }

        deserializer.deserialize_any(InputVisitor)
    }
}

/// Locked metadata for a node. Only [`Self::rev`] is consumed by the rest
/// of the crate; other coordinates from the JSON (`owner`, `repo`, `type`,
/// `ref`, `narHash`, ...) are ignored on parse.
#[derive(Debug, Deserialize, Clone)]
pub(crate) struct Locked {
    rev: Option<String>,
}

impl Locked {
    fn rev(&self) -> Result<String, FlakeEditError> {
        self.rev
            .clone()
            .ok_or_else(|| FlakeEditError::LockError("Locked node has no rev.".into()))
    }
}

/// Original (pre-lock) reference for a node, as written in the source flake.
#[derive(Debug, Deserialize)]
pub(crate) struct Original {
    owner: Option<String>,
    repo: Option<String>,
    #[serde(rename = "type")]
    node_type: String,
    #[serde(rename = "ref")]
    ref_field: Option<String>,
    url: Option<String>,
}

impl Original {
    /// Reconstruct a flake URL from the original reference. Returns `None`
    /// when the type is forge-shaped (`github`, `gitlab`, `sourcehut`) but
    /// `owner` or `repo` is missing.
    fn to_flake_url(&self) -> Option<String> {
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

    /// Load `flake.lock` from the current directory.
    pub fn from_default_path() -> Result<Self, FlakeEditError> {
        let path = PathBuf::from(Self::LOCK);
        Self::from_file(path)
    }

    /// Load and parse a lockfile from `path`.
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, FlakeEditError> {
        let mut file = File::open(path)?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        Self::read_from_str(&contents)
    }

    /// Parse lockfile JSON from `str`.
    pub fn read_from_str(str: &str) -> Result<Self, FlakeEditError> {
        Ok(serde_json::from_str(str)?)
    }

    /// Name of the root node.
    pub fn root(&self) -> &str {
        &self.root
    }

    /// Resolve an input path (sequence of *input names*) to the node key it
    /// ultimately points at, walking the lock tree from `root`.
    ///
    /// Lockfile `Indirect` entries are `follows` paths of input names rooted at
    /// the lock root, not node keys, so encountering one restarts the walk from
    /// root with that path plus any remaining segments.
    fn resolve_input_path<S: AsRef<str>>(&self, segments: &[S]) -> Result<String, FlakeEditError> {
        // Bound recursion: a valid lock file has no follows cycles, but we
        // still guard against malformed input.
        const MAX_HOPS: usize = 64;
        self.resolve_input_path_inner(segments, MAX_HOPS)
    }

    fn resolve_input_path_inner<S: AsRef<str>>(
        &self,
        segments: &[S],
        budget: usize,
    ) -> Result<String, FlakeEditError> {
        if budget == 0 {
            return Err(FlakeEditError::LockError(
                "Cycle while resolving follows path.".into(),
            ));
        }
        if segments.is_empty() {
            return Err(FlakeEditError::LockError("Empty input path.".into()));
        }

        let mut current_key = self.root.clone();
        let mut current_node = self
            .nodes
            .get(self.root())
            .ok_or(FlakeEditError::LockMissingRoot)?;

        for (i, segment) in segments.iter().enumerate() {
            let segment = segment.as_ref();
            let inputs = current_node.inputs.as_ref().ok_or_else(|| {
                if i == 0 {
                    FlakeEditError::LockError("Could not resolve root.".into())
                } else {
                    let prefix: Vec<_> = segments[..i].iter().map(|s| s.as_ref()).collect();
                    FlakeEditError::LockError(format!(
                        "Input '{}' has no sub-inputs.",
                        prefix.join(".")
                    ))
                }
            })?;

            let resolved = inputs.get(segment).ok_or_else(|| {
                let prefix: Vec<_> = segments[..=i].iter().map(|s| s.as_ref()).collect();
                FlakeEditError::LockError(format!(
                    "Input '{}' not found in lock file.",
                    prefix.join(".")
                ))
            })?;

            match resolved {
                Input::Direct(node_key) => {
                    current_key = node_key.clone();
                }
                Input::Indirect(Some(follows_path)) => {
                    let mut new_path: Vec<String> = follows_path
                        .segments()
                        .iter()
                        .map(|s| s.as_str().to_string())
                        .collect();
                    new_path.extend(segments[i + 1..].iter().map(|s| s.as_ref().to_string()));
                    return self.resolve_input_path_inner(&new_path, budget - 1);
                }
                Input::Indirect(None) => {
                    let prefix: Vec<_> = segments[..=i].iter().map(|s| s.as_ref()).collect();
                    return Err(FlakeEditError::LockError(format!(
                        "Input '{}' has no follows target.",
                        prefix.join(".")
                    )));
                }
            }

            if i + 1 < segments.len() {
                current_node = self.nodes.get(&current_key).ok_or_else(|| {
                    let prefix: Vec<_> = segments[..=i].iter().map(|s| s.as_ref()).collect();
                    FlakeEditError::LockError(format!(
                        "Could not find node '{}' for input '{}'.",
                        current_key,
                        prefix.join(".")
                    ))
                })?;
            }
        }

        Ok(current_key)
    }

    /// Resolve `id` (a dotted attribute path) to its locked revision.
    ///
    /// # Errors
    ///
    /// Returns [`FlakeEditError::LockError`] if `id` is malformed or does not
    /// resolve to a node with a `rev` field.
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

    /// Rendered nested input paths (e.g. `naersk.nixpkgs`,
    /// `naersk.flake-utils`) suitable for shell completion.
    pub fn nested_input_paths(&self) -> Vec<String> {
        self.nested_inputs()
            .into_iter()
            .map(|input| input.path.to_string())
            .collect()
    }

    /// All nested inputs reachable from the root, with their existing
    /// follows targets.
    ///
    /// Walks `flake.lock` recursively from the root, emitting one entry per
    /// `inputs.X` on any descendant node and building the path
    /// segment-by-segment. Capped at [`NESTED_INPUTS_MAX_DEPTH`]. Cycles in
    /// the node graph are broken by a visited set keyed on node name.
    ///
    /// Output is sorted by path for stable emission order.
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
                // Indirect top-level inputs have no sub-inputs to descend
                // into. Their follows edges still appear via the recursive
                // walker on the `Direct` siblings that own them.
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

    /// Descend through `node_name`, emitting one [`NestedInput`] per declared
    /// sub-input up to [`NESTED_INPUTS_MAX_DEPTH`].
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
                Input::Indirect(Some(target)) => (Some(target.clone()), None, None),
                // Empty `[]` declarations have no follows target. Emit
                // the input with `follows: None` so the path is still
                // visible to the UI.
                Input::Indirect(None) => (None, None, None),
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

/// Maximum recursion depth for [`FlakeLock::nested_inputs`]. Backstops
/// pathological cycles in malformed lockfiles.
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
    /// The lockfile's top-level `"version"` field is not validated. A
    /// wildly unsupported version (e.g. `99`) must still parse cleanly
    /// so this crate can read whatever shape Nix produces.
    #[test]
    fn parse_ignores_unknown_version() {
        let lock = r#"{
  "nodes": { "root": { "inputs": {} } },
  "root": "root",
  "version": 99
}"#;
        FlakeLock::read_from_str(lock).expect("unknown version must still parse");
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
    fn rev_for_sub_input_path_missing_parent_returns_error() {
        let minimal_lock = minimal_lock();
        let parsed_lock =
            FlakeLock::read_from_str(minimal_lock).expect("Should be parsed correctly.");
        assert!(parsed_lock.rev_for("browseros.nixpkgs").is_err());
    }

    #[test]
    fn rev_for_sub_input_path_resolves() {
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

    /// An `Indirect` follows path names *inputs* from the lock root, not node
    /// keys. Resolution must go through `root.inputs`, which may map e.g.
    /// `nixpkgs` to a node keyed `nixpkgs_2`.
    #[test]
    fn rev_for_indirect_resolves_via_root_inputs() {
        let lock = r#"{
  "nodes": {
    "nixpkgs_2": {
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "treefmt-nix": {
      "inputs": { "nixpkgs": ["nixpkgs"] },
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "root": {
      "inputs": { "nixpkgs": "nixpkgs_2", "treefmt-nix": "treefmt-nix" }
    }
  },
  "root": "root",
  "version": 7
}"#;
        let parsed = FlakeLock::read_from_str(lock).unwrap();
        // treefmt-nix.nixpkgs follows ["nixpkgs"], i.e. root.inputs.nixpkgs,
        // which is node "nixpkgs_2". There is no node literally named "nixpkgs".
        assert_eq!(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            parsed
                .rev_for("treefmt-nix.nixpkgs")
                .expect("indirect follows must resolve through root.inputs, not by node name")
        );
    }

    /// A multi-segment follows path like `["crane", "nixpkgs"]` must be walked
    /// segment-by-segment from root, since each hop can map to a renamed node key.
    #[test]
    fn rev_for_indirect_multi_segment_path() {
        let lock = r#"{
  "nodes": {
    "nixpkgs": {
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "1111111111111111111111111111111111111111", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "nixpkgs_2": {
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "2222222222222222222222222222222222222222", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "crane": {
      "inputs": { "nixpkgs": "nixpkgs_2" },
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "cccccccccccccccccccccccccccccccccccccccc", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "devshell": {
      "inputs": { "nixpkgs": ["crane", "nixpkgs"] },
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "dddddddddddddddddddddddddddddddddddddddd", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "root": {
      "inputs": { "nixpkgs": "nixpkgs", "crane": "crane", "devshell": "devshell" }
    }
  },
  "root": "root",
  "version": 7
}"#;
        let parsed = FlakeLock::read_from_str(lock).unwrap();
        // devshell.nixpkgs follows ["crane","nixpkgs"] -> node "nixpkgs_2" (rev 222...),
        // NOT root's "nixpkgs" node (rev 111...).
        assert_eq!(
            "2222222222222222222222222222222222222222",
            parsed
                .rev_for("devshell.nixpkgs")
                .expect("multi-segment indirect follows must be walked from root")
        );
    }

    /// Walk every `inputs` map and collect each `Indirect` follows target as
    /// a vector of segment strings. Used by the fixture parse tests below.
    fn collect_indirect_targets(lock: &FlakeLock) -> Vec<(String, String, Vec<String>)> {
        let mut out: Vec<(String, String, Vec<String>)> = Vec::new();
        for (node_name, node) in &lock.nodes {
            let Some(inputs) = node.inputs.as_ref() else {
                continue;
            };
            for (input_name, input_ref) in inputs {
                if let Input::Indirect(Some(path)) = input_ref {
                    let segs: Vec<String> = path
                        .segments()
                        .iter()
                        .map(|s| s.as_str().to_string())
                        .collect();
                    out.push((node_name.clone(), input_name.clone(), segs));
                }
            }
        }
        out.sort();
        out
    }

    /// `depth_upstream_redundant_depth3.flake.lock` has three `Indirect`
    /// entries forming the upstream-propagation chain: `omnibus.nixpkgs`
    /// follows `["nixpkgs"]`, the depth-2 follows `["omnibus", "nixpkgs"]`,
    /// and the depth-3 follows `["omnibus", "flops", "nixpkgs"]`.
    #[test]
    fn fixture_depth_upstream_redundant_depth3_parses_indirects() {
        let lock_text =
            std::fs::read_to_string("tests/fixtures/depth_upstream_redundant_depth3.flake.lock")
                .expect("fixture present");
        let lock = FlakeLock::read_from_str(&lock_text).expect("fixture parses");
        let mut segs_only: Vec<Vec<String>> = collect_indirect_targets(&lock)
            .into_iter()
            .map(|(_, _, segs)| segs)
            .collect();
        segs_only.sort();
        assert_eq!(
            segs_only,
            vec![
                vec!["nixpkgs".to_string()],
                vec![
                    "omnibus".to_string(),
                    "flops".to_string(),
                    "nixpkgs".to_string()
                ],
                vec!["omnibus".to_string(), "nixpkgs".to_string()],
            ],
            "Indirect entries must be decoded with their full structural depth",
        );
    }

    /// `depth_upstream_partial.flake.lock` covers a deeper mix of Indirect
    /// shapes; verify each one is decoded into a non-empty `AttrPath` whose
    /// segments are valid Nix names (no embedded quotes / control chars).
    #[test]
    fn fixture_depth_upstream_partial_parses_indirects() {
        let lock_text = std::fs::read_to_string("tests/fixtures/depth_upstream_partial.flake.lock")
            .expect("fixture present");
        let lock = FlakeLock::read_from_str(&lock_text).expect("fixture parses");
        let entries = collect_indirect_targets(&lock);
        assert!(
            entries.len() >= 3,
            "fixture has at least three Indirect arrays, got {}",
            entries.len()
        );
        for (node, input, segs) in &entries {
            assert!(
                !segs.is_empty(),
                "{node}.{input}: Indirect path must be non-empty",
            );
            for seg in segs {
                assert!(
                    !seg.is_empty() && !seg.contains('"'),
                    "{node}.{input}: segment `{seg}` must be a valid Nix name",
                );
            }
        }
    }

    /// `dot_ancestor_cycle.flake.lock` exercises the dotted-segment case:
    /// the lockfile node `hls-1.10` is reachable through the typed
    /// `AttrPath`, even though a literal dot in the segment forces source-
    /// form quoting at the `flake.nix` boundary.
    #[test]
    fn fixture_dot_ancestor_cycle_parses_indirects_with_dotted_node() {
        let lock_text = std::fs::read_to_string("tests/fixtures/dot_ancestor_cycle.flake.lock")
            .expect("fixture present");
        let lock = FlakeLock::read_from_str(&lock_text).expect("fixture parses");
        // Direct walk: the dotted node `hls-1.10` exists and its
        // `Indirect` `["helper"]` entry is decoded as a one-segment path.
        let hls = lock.nodes.get("hls-1.10").expect("hls-1.10 node");
        let inputs = hls.inputs.as_ref().expect("hls-1.10 has inputs");
        match inputs.get("helper").expect("helper input present") {
            Input::Indirect(Some(path)) => {
                let segs: Vec<&str> = path.segments().iter().map(|s| s.as_str()).collect();
                assert_eq!(segs, vec!["helper"]);
            }
            Input::Indirect(None) => panic!("expected Indirect(Some), got Indirect(None)"),
            Input::Direct(name) => panic!("expected Indirect, got Direct({name})"),
        }
    }

    /// Empty `[]` follows arrays mark an input whose follows chain has
    /// been overridden away (e.g. a lockfile entry `inputs.flake-compat
    /// = []`). The deserializer must accept them and store them as
    /// `Indirect(None)`.
    #[test]
    fn indirect_empty_array_is_accepted_as_none() {
        let lock = r#"{
  "nodes": {
    "child": {
      "inputs": { "disabled": [] },
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "x", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "root": { "inputs": { "child": "child" } }
  },
  "root": "root",
  "version": 7
}"#;
        let parsed = FlakeLock::read_from_str(lock).expect("empty Indirect must parse");
        let child = parsed.nodes.get("child").expect("child node");
        let inputs = child.inputs.as_ref().expect("child has inputs");
        match inputs.get("disabled").expect("disabled input present") {
            Input::Indirect(None) => {}
            other => panic!("expected Indirect(None), got {other:?}"),
        }
    }

    /// A top-level input (`nix`) whose nested inputs map mixes
    /// [`Input::Direct`] references, non-empty
    /// [`Input::Indirect`] arrays (`["nixpkgs"]`), and empty `[]`
    /// declarations must parse cleanly. [`FlakeLock::nested_inputs`]
    /// emits one entry per declaration; entries built from `[]` carry
    /// `follows: None`.
    #[test]
    fn nested_inputs_handles_mixed_direct_indirect_and_empty() {
        let lock = r#"{
  "nodes": {
    "flake-parts": {
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "fp", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "nix": {
      "inputs": {
        "flake-compat": [],
        "flake-parts": "flake-parts",
        "nixpkgs": ["nixpkgs"]
      },
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "n", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "nixpkgs": {
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "np", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "root": { "inputs": { "nix": "nix", "nixpkgs": "nixpkgs" } }
  },
  "root": "root",
  "version": 7
}"#;
        let parsed = FlakeLock::read_from_str(lock).expect("mixed-shape lock parses");
        let nested = parsed.nested_inputs();
        let by_path: std::collections::HashMap<String, &NestedInput> =
            nested.iter().map(|n| (n.path.to_string(), n)).collect();

        let disabled = by_path
            .get("nix.flake-compat")
            .expect("empty Indirect emitted as nested input");
        assert!(
            disabled.follows.is_none(),
            "empty `[]` must surface as follows: None, got {:?}",
            disabled.follows
        );

        let resolved = by_path
            .get("nix.nixpkgs")
            .expect("non-empty Indirect emitted as nested input");
        assert_eq!(
            resolved.follows.as_ref().map(|p| p.to_string()),
            Some("nixpkgs".to_string()),
            "non-empty Indirect must surface its follows target",
        );
    }
}
