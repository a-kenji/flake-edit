//! Module for handling auto-follows functionality during input addition.
//!
//! This module provides utilities to:
//! - Fetch flake metadata from remote URIs
//! - Extract dependency inputs from flake metadata
//! - Find matching follow candidates between a dependency and local inputs

use serde::Deserialize;
use std::collections::HashMap;
use std::process::Command;
use thiserror::Error;

use crate::change::FollowSpec;

#[derive(Error, Debug)]
pub enum FollowsError {
    #[error("Failed to execute nix command: {0}")]
    NixCommandFailed(std::io::Error),

    #[error("Nix command returned error: {0}")]
    NixCommandError(String),

    #[error("Failed to parse nix output: {0}")]
    ParseError(#[from] serde_json::Error),

    #[error("No inputs found in dependency")]
    NoInputsFound,
}

/// Metadata returned by `nix flake metadata --json`
#[derive(Debug, Clone, Deserialize)]
pub struct FlakeMetadata {
    #[allow(dead_code)]
    pub description: Option<String>,
    pub locks: FlakeLocks,
}

/// Lock information from flake metadata
#[derive(Debug, Clone, Deserialize)]
pub struct FlakeLocks {
    pub nodes: HashMap<String, MetadataNode>,
    pub root: String,
    #[allow(dead_code)]
    pub version: u8,
}

/// A node in the flake lock graph
#[derive(Debug, Clone, Deserialize)]
pub struct MetadataNode {
    pub inputs: Option<HashMap<String, MetadataInput>>,
    #[allow(dead_code)]
    pub locked: Option<serde_json::Value>,
    #[allow(dead_code)]
    pub original: Option<serde_json::Value>,
}

/// Input reference in a metadata node - can be direct or indirect (follows)
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum MetadataInput {
    /// Direct reference to another node
    Direct(String),
    /// Indirect reference (follows) - path through the graph
    Indirect(Vec<String>),
}

/// Represents a potential follow match between a new dependency's input
/// and an existing input in the current flake.
#[derive(Debug, Clone, PartialEq)]
pub struct FollowCandidate {
    /// The input name in the new dependency (e.g., "nixpkgs")
    pub dep_input: String,
    /// The matching input name in the current flake (e.g., "nixpkgs")
    pub local_input: String,
}

impl FollowCandidate {
    pub fn new(dep_input: impl Into<String>, local_input: impl Into<String>) -> Self {
        Self {
            dep_input: dep_input.into(),
            local_input: local_input.into(),
        }
    }

    /// Convert to a FollowSpec for use in Change::Add
    pub fn to_follow_spec(&self) -> FollowSpec {
        FollowSpec::new(&self.dep_input, &self.local_input)
    }
}

/// Fetch flake metadata for a given URI using `nix flake metadata --json`.
///
/// This calls the nix command to get metadata about a remote flake,
/// which includes its lock information with all inputs.
pub fn fetch_flake_metadata(uri: &str) -> Result<FlakeMetadata, FollowsError> {
    tracing::debug!("Fetching flake metadata for: {}", uri);

    let output = Command::new("nix")
        .args(["flake", "metadata", "--json", "--no-write-lock-file", uri])
        .output()
        .map_err(FollowsError::NixCommandFailed)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(FollowsError::NixCommandError(stderr.to_string()));
    }

    let metadata: FlakeMetadata = serde_json::from_slice(&output.stdout)?;
    tracing::debug!("Successfully fetched metadata for: {}", uri);
    Ok(metadata)
}

/// Extract the list of input names from flake metadata.
///
/// Returns the direct inputs of the root node - these are the inputs
/// that the dependency flake declares in its flake.nix.
pub fn get_dependency_inputs(metadata: &FlakeMetadata) -> Vec<String> {
    let root_name = &metadata.locks.root;

    metadata
        .locks
        .nodes
        .get(root_name)
        .and_then(|node| node.inputs.as_ref())
        .map(|inputs| {
            inputs
                .iter()
                .filter_map(|(name, input)| {
                    // Only include direct inputs, not follows references
                    match input {
                        MetadataInput::Direct(_) => Some(name.clone()),
                        MetadataInput::Indirect(_) => None,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Find potential follow candidates by matching dependency inputs with local inputs.
///
/// Uses exact name matching only - returns candidates where the dependency
/// has an input with the same name as a local input.
pub fn find_follow_candidates(
    dep_inputs: &[String],
    local_inputs: &[String],
) -> Vec<FollowCandidate> {
    let mut candidates = Vec::new();

    for dep_input in dep_inputs {
        // Check if there's a local input with the same name (exact match)
        if local_inputs.contains(dep_input) {
            candidates.push(FollowCandidate::new(dep_input, dep_input));
        }
    }

    // Sort for consistent output
    candidates.sort_by(|a, b| a.dep_input.cmp(&b.dep_input));
    candidates
}

/// Convert follow candidates to FollowSpecs.
///
/// Takes a list of candidates and converts them to FollowSpecs that can
/// be used in Change::Add.
pub fn candidates_to_specs(candidates: &[FollowCandidate]) -> Vec<FollowSpec> {
    candidates.iter().map(|c| c.to_follow_spec()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_follow_candidate_to_spec() {
        let candidate = FollowCandidate::new("nixpkgs", "nixpkgs");
        let spec = candidate.to_follow_spec();
        assert_eq!(spec.from, "nixpkgs");
        assert_eq!(spec.to, "nixpkgs");
    }

    #[test]
    fn test_find_follow_candidates_exact_match() {
        let dep_inputs = vec!["nixpkgs".to_string(), "flake-utils".to_string()];
        let local_inputs = vec!["nixpkgs".to_string(), "crane".to_string()];

        let candidates = find_follow_candidates(&dep_inputs, &local_inputs);

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].dep_input, "nixpkgs");
        assert_eq!(candidates[0].local_input, "nixpkgs");
    }

    #[test]
    fn test_find_follow_candidates_multiple_matches() {
        let dep_inputs = vec![
            "nixpkgs".to_string(),
            "flake-utils".to_string(),
            "systems".to_string(),
        ];
        let local_inputs = vec![
            "nixpkgs".to_string(),
            "flake-utils".to_string(),
            "crane".to_string(),
        ];

        let candidates = find_follow_candidates(&dep_inputs, &local_inputs);

        assert_eq!(candidates.len(), 2);
        // Should be sorted
        assert_eq!(candidates[0].dep_input, "flake-utils");
        assert_eq!(candidates[1].dep_input, "nixpkgs");
    }

    #[test]
    fn test_find_follow_candidates_no_matches() {
        let dep_inputs = vec!["rust-overlay".to_string()];
        let local_inputs = vec!["nixpkgs".to_string()];

        let candidates = find_follow_candidates(&dep_inputs, &local_inputs);

        assert!(candidates.is_empty());
    }

    #[test]
    fn test_candidates_to_specs() {
        let candidates = vec![
            FollowCandidate::new("nixpkgs", "nixpkgs"),
            FollowCandidate::new("flake-utils", "flake-utils"),
        ];

        let specs = candidates_to_specs(&candidates);

        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].from, "nixpkgs");
        assert_eq!(specs[0].to, "nixpkgs");
    }

    #[test]
    fn test_get_dependency_inputs_filters_follows() {
        // Simulate metadata where some inputs are follows references
        let json = r#"{
            "locks": {
                "nodes": {
                    "root": {
                        "inputs": {
                            "nixpkgs": "nixpkgs_locked",
                            "systems": ["nixpkgs", "systems"]
                        }
                    },
                    "nixpkgs_locked": {}
                },
                "root": "root",
                "version": 7
            }
        }"#;

        let metadata: FlakeMetadata = serde_json::from_str(json).unwrap();
        let inputs = get_dependency_inputs(&metadata);

        // Should only include "nixpkgs", not "systems" which is a follows reference
        assert_eq!(inputs.len(), 1);
        assert!(inputs.contains(&"nixpkgs".to_string()));
    }
}
