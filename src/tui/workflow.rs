//! Workflow state and logic for TUI interactions.
//!
//! This module contains the pure workflow logic separated from screen handling:
//! - Workflow data types (Add, Change, Remove, etc.)
//! - Result types returned by workflows
//! - Helper functions for URI parsing and diff computation

use std::collections::HashMap;

use nix_uri::FlakeRef;
use nix_uri::urls::UrlWrapper;

use crate::change::Change;
use crate::diff::Diff;
use crate::edit::FlakeEdit;
use crate::lock::NestedInput;

/// Result from single-select including selected item and whether diff preview is enabled
#[derive(Debug, Clone)]
pub struct SingleSelectResult {
    pub item: String,
    pub show_diff: bool,
}

/// Result from multi-select including selected items and whether diff preview is enabled
#[derive(Debug, Clone)]
pub struct MultiSelectResultData {
    pub items: Vec<String>,
    pub show_diff: bool,
}

/// Result from confirmation screen
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmResultAction {
    Apply,
    Back,
    Exit,
}

/// Phase within the Add workflow
#[derive(Debug, Clone, PartialEq)]
pub enum AddPhase {
    Uri,
    Id,
}

/// Phase within the Follow workflow
#[derive(Debug, Clone, PartialEq)]
pub enum FollowPhase {
    /// Select the nested input path (e.g., "crane.nixpkgs")
    SelectInput,
    /// Select the target to follow (e.g., "nixpkgs")
    SelectTarget,
}

/// Result of calling update()
#[derive(Debug, Clone)]
pub enum UpdateResult {
    /// Keep processing events
    Continue,
    /// Workflow complete
    Done,
    /// Workflow cancelled
    Cancelled,
}

/// Result returned by run() - depends on the workflow type
#[derive(Debug, Clone)]
pub enum AppResult {
    /// Result from Add/Change/Remove workflows
    Change(Change),
    /// Result from SelectOne workflow
    SingleSelect(SingleSelectResult),
    /// Result from SelectMany workflow
    MultiSelect(MultiSelectResultData),
    /// Result from ConfirmOnly workflow
    Confirm(ConfirmResultAction),
}

/// Workflow-specific data tracking the state of the current operation
#[derive(Debug, Clone)]
pub enum WorkflowData {
    Add {
        phase: AddPhase,
        uri: Option<String>,
        id: Option<String>,
    },
    Change {
        selected_input: Option<String>,
        uri: Option<String>,
        input_uris: HashMap<String, String>,
        all_inputs: Vec<String>,
    },
    Remove {
        selected_inputs: Vec<String>,
        all_inputs: Vec<String>,
    },
    SelectOne {
        selected_input: Option<String>,
    },
    SelectMany {
        selected_inputs: Vec<String>,
    },
    ConfirmOnly {
        action: Option<ConfirmResultAction>,
    },
    Follow {
        phase: FollowPhase,
        /// The selected nested input path (e.g., "crane.nixpkgs")
        selected_input: Option<String>,
        /// The selected target to follow (e.g., "nixpkgs")
        selected_target: Option<String>,
        /// All available nested inputs with their follows info
        nested_inputs: Vec<NestedInput>,
        /// All available top-level inputs (possible targets)
        top_level_inputs: Vec<String>,
    },
}

impl WorkflowData {
    /// Build a Change based on the current workflow state.
    pub fn build_change(&self) -> Change {
        match self {
            WorkflowData::Add { id, uri, .. } => Change::Add {
                id: id.clone(),
                uri: uri.clone(),
                flake: true,
            },
            WorkflowData::Change {
                selected_input,
                uri,
                ..
            } => Change::Change {
                id: selected_input.clone(),
                uri: uri.clone(),
                ref_or_rev: None,
            },
            WorkflowData::Remove {
                selected_inputs, ..
            } => {
                if selected_inputs.is_empty() {
                    Change::None
                } else {
                    Change::Remove {
                        ids: selected_inputs.iter().map(|s| s.clone().into()).collect(),
                    }
                }
            }
            // Standalone workflows don't produce Changes
            WorkflowData::SelectOne { .. }
            | WorkflowData::SelectMany { .. }
            | WorkflowData::ConfirmOnly { .. } => Change::None,
            WorkflowData::Follow {
                selected_input,
                selected_target,
                ..
            } => {
                if let (Some(input), Some(target)) = (selected_input, selected_target) {
                    Change::Follows {
                        input: input.clone().into(),
                        target: target.clone(),
                    }
                } else {
                    Change::None
                }
            }
        }
    }
}

/// Parse a URI and try to infer the ID from it.
///
/// Returns (inferred_id, normalized_uri) where normalized_uri is the parsed
/// string representation if valid, or the original URI if parsing failed.
pub fn parse_uri_and_infer_id(uri: &str) -> (Option<String>, String) {
    let flake_ref: Result<FlakeRef, _> = UrlWrapper::convert_or_parse(uri);
    if let Ok(flake_ref) = flake_ref {
        let parsed_uri = flake_ref.to_string();
        let final_uri = if parsed_uri.is_empty() || parsed_uri == "none" {
            uri.to_string()
        } else {
            parsed_uri
        };
        (flake_ref.id(), final_uri)
    } else {
        (None, uri.to_string())
    }
}

/// Compute a unified diff between the original flake text and the result of applying a change.
pub fn compute_diff(flake_text: &str, change: &Change) -> String {
    // Return empty string for None change (no preview possible)
    if matches!(change, Change::None) {
        return String::new();
    }

    let Ok(mut edit) = FlakeEdit::from_text(flake_text) else {
        return "Error parsing flake".to_string();
    };

    let new_text = match edit.apply_change(change.clone()) {
        Ok(Some(text)) => text,
        Ok(None) => flake_text.to_string(),
        Err(e) => return format!("Error: {e}"),
    };

    Diff::new(flake_text, &new_text).to_string_plain()
}
