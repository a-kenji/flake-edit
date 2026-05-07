use std::collections::HashMap;

use crate::change::Change;
use crate::error::FlakeEditError;
use crate::input::{Follows, Input};
use crate::validate;
use crate::walk::Walker;

pub struct FlakeEdit {
    walker: Walker,
}

#[derive(Default, Debug)]
pub enum Outputs {
    #[default]
    None,
    Multiple(Vec<String>),
    Any(Vec<String>),
}

pub type InputMap = HashMap<String, Input>;

/// Sorted input ids from `inputs`.
pub fn sorted_input_ids(inputs: &InputMap) -> Vec<&String> {
    let mut keys: Vec<_> = inputs.keys().collect();
    keys.sort();
    keys
}

#[derive(Default, Debug)]
pub enum OutputChange {
    #[default]
    None,
    Add(String),
    Remove(String),
}

/// Result of applying a [`Change`].
///
/// `text` is the new flake source, or `None` for a no-op (e.g. an
/// already-existing follows declaration).
#[derive(Debug, Default)]
pub struct ApplyOutcome {
    pub text: Option<String>,
}

impl FlakeEdit {
    pub fn from_text(stream: &str) -> Result<Self, FlakeEditError> {
        let validation = validate::validate(stream);
        if validation.has_errors() {
            return Err(FlakeEditError::Validation(validation.errors));
        }

        let walker = Walker::new(stream);
        Ok(Self { walker })
    }

    pub fn source_text(&self) -> String {
        self.walker.root.to_string()
    }

    pub fn curr_list(&self) -> &InputMap {
        &self.walker.inputs
    }

    /// Re-walk the source and return the freshly populated input map. Use
    /// [`Self::curr_list`] to read the cached map without re-walking.
    pub fn list(&mut self) -> &InputMap {
        self.walker.inputs.clear();
        // Walk returns Ok(None) when no changes are made (expected for listing)
        assert!(self.walker.walk(&Change::None).ok().flatten().is_none());
        &self.walker.inputs
    }
    /// Apply `change` and return the resulting [`ApplyOutcome`].
    ///
    /// Some edits require multiple walker passes. This method drives them all.
    /// A fatal validation failure surfaces as
    /// [`FlakeEditError::Validation`].
    ///
    /// # Errors
    ///
    /// Returns [`FlakeEditError`] if the underlying walker fails or the change
    /// is rejected (e.g. [`FlakeEditError::DuplicateInput`],
    /// [`FlakeEditError::InputNotFound`]).
    pub fn apply_change(&mut self, change: Change) -> Result<ApplyOutcome, FlakeEditError> {
        let text = self.apply_change_text(change)?;
        Ok(ApplyOutcome { text })
    }

    fn apply_change_text(&mut self, change: Change) -> Result<Option<String>, FlakeEditError> {
        match change {
            Change::None => Ok(None),
            Change::Add { .. } => {
                // Check for duplicate input before adding
                if let Some(input_id) = change.id() {
                    self.ensure_inputs_populated()?;

                    let input_id_string = input_id.input().as_str().to_string();
                    if self.walker.inputs.contains_key(&input_id_string) {
                        return Err(FlakeEditError::DuplicateInput(input_id_string));
                    }
                }

                if let Some(maybe_changed_node) = self.walker.walk(&change.clone())? {
                    let outputs = self.walker.list_outputs()?;
                    match outputs {
                        Outputs::Multiple(out) => {
                            let id = change.id().unwrap().input().as_str().to_string();
                            if !out.contains(&id) {
                                self.walker.root = maybe_changed_node.clone();
                                if let Some(maybe_changed_node) =
                                    self.walker.change_outputs(OutputChange::Add(id))?
                                {
                                    return Ok(Some(maybe_changed_node.to_string()));
                                }
                            }
                        }
                        Outputs::None | Outputs::Any(_) => {}
                    }
                    Ok(Some(maybe_changed_node.to_string()))
                } else {
                    self.walker.add_toplevel = true;
                    let maybe_changed_node = self.walker.walk(&change)?;
                    Ok(maybe_changed_node.map(|n| n.to_string()))
                }
            }
            Change::Remove { .. } => {
                self.ensure_inputs_populated()?;

                let id = change.id().unwrap();
                // Outputs-lambda strip and orphan-follows scrubbing only
                // run for a top-level input remove. A depth-N follows id
                // shares its first segment with a still-present input;
                // running the cleanup there would strip that input from
                // the outputs lambda.
                let is_toplevel_remove = id.follows().is_none();
                let removed_id = id.input().as_str().to_string();

                // If we remove a node, it could be a flat structure,
                // we want to remove all of the references to its toplevel.
                let mut res = None;
                while let Some(changed_node) = self.walker.walk(&change)? {
                    if res == Some(changed_node.clone()) {
                        break;
                    }
                    res = Some(changed_node.clone());
                    self.walker.root = changed_node.clone();
                }

                if is_toplevel_remove {
                    let outputs = self.walker.list_outputs()?;
                    match outputs {
                        Outputs::Multiple(out) | Outputs::Any(out) => {
                            if out.contains(&removed_id)
                                && let Some(changed_node) = self
                                    .walker
                                    .change_outputs(OutputChange::Remove(removed_id.clone()))?
                            {
                                res = Some(changed_node.clone());
                                self.walker.root = changed_node.clone();
                            }
                        }
                        Outputs::None => {}
                    }

                    let orphaned_follows = self.collect_orphaned_follows(&removed_id);
                    for orphan_change in orphaned_follows {
                        while let Some(changed_node) = self.walker.walk(&orphan_change)? {
                            if res == Some(changed_node.clone()) {
                                break;
                            }
                            res = Some(changed_node.clone());
                            self.walker.root = changed_node.clone();
                        }
                    }
                }

                Ok(res.map(|n| n.to_string()))
            }
            Change::Follows { ref input, .. } => {
                self.ensure_inputs_populated()?;

                let parent_id = input.input().as_str();
                if !self.walker.inputs.contains_key(parent_id) {
                    return Err(FlakeEditError::InputNotFound(parent_id.to_string()));
                }

                if let Some(maybe_changed_node) = self.walker.walk(&change)? {
                    Ok(Some(maybe_changed_node.to_string()))
                } else {
                    Ok(None)
                }
            }
            Change::Change { .. } => {
                if let Some(input_id) = change.id() {
                    self.ensure_inputs_populated()?;

                    let input_id_string = input_id.input().as_str().to_string();
                    if !self.walker.inputs.contains_key(&input_id_string) {
                        return Err(FlakeEditError::InputNotFound(input_id_string));
                    }
                }

                if let Some(maybe_changed_node) = self.walker.walk(&change)? {
                    Ok(Some(maybe_changed_node.to_string()))
                } else {
                    Ok(None)
                }
            }
        }
    }

    pub fn walker(&self) -> &Walker {
        &self.walker
    }

    /// Walk once if the inputs map is empty.
    fn ensure_inputs_populated(&mut self) -> Result<(), FlakeEditError> {
        if self.walker.inputs.is_empty() {
            let _ = self.walker.walk(&Change::None)?;
        }
        Ok(())
    }

    /// Collect [`Change::Remove`]s for follows declarations whose target
    /// top-level segment matches `removed_id`.
    fn collect_orphaned_follows(&self, removed_id: &str) -> Vec<Change> {
        let mut orphaned = Vec::new();
        for (input_id, input) in &self.walker.inputs {
            for follows in input.follows() {
                if let Follows::Indirect {
                    path,
                    target: Some(target),
                } = follows
                {
                    // target is the RHS of `follows = "..."`. Match when its
                    // top-level segment is the removed input. Empty targets
                    // (`follows = ""`) have nothing to follow and cannot
                    // dangle.
                    if target.first().as_str() == removed_id {
                        let path_str = format!("{}.{}", input_id, path);
                        let Ok(change_id) = crate::change::ChangeId::parse(&path_str) else {
                            continue;
                        };
                        orphaned.push(Change::Remove {
                            ids: vec![change_id],
                        });
                    }
                }
            }
        }
        orphaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn already_follows_is_noop() {
        let flake = r#"{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
    crane = {
      url = "github:ipetkov/crane";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };
  outputs = { ... }: { };
}"#;
        let mut fe = FlakeEdit::from_text(flake).unwrap();
        let original = fe.source_text();
        let change = Change::Follows {
            input: crate::change::ChangeId::parse("crane.nixpkgs").unwrap(),
            target: crate::follows::AttrPath::parse("nixpkgs").unwrap(),
        };
        let result = fe.apply_change(change).unwrap();
        // Walker signals a no-op as either the unchanged text or `None`.
        // Both are acceptable here.
        if let Some(text) = result.text {
            assert_eq!(text, original, "text should be unchanged");
        }
    }

    #[test]
    fn new_follows_succeeds() {
        let flake = r#"{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
    crane = {
      url = "github:ipetkov/crane";
    };
  };
  outputs = { ... }: { };
}"#;
        let mut fe = FlakeEdit::from_text(flake).unwrap();
        let change = Change::Follows {
            input: crate::change::ChangeId::parse("crane.nixpkgs").unwrap(),
            target: crate::follows::AttrPath::parse("nixpkgs").unwrap(),
        };
        let result = fe.apply_change(change);
        assert!(result.is_ok(), "expected Ok, got: {:?}", result);
        let text = result.unwrap().text.unwrap();
        assert!(text.contains("inputs.nixpkgs.follows = \"nixpkgs\""));
    }
}
