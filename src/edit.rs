use std::collections::HashMap;

use crate::change::Change;
use crate::error::FlakeEditError;
use crate::input::{Follows, Input};
use crate::walk::Walker;

pub struct FlakeEdit {
    changes: Vec<Change>,
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

#[derive(Default, Debug)]
pub enum OutputChange {
    #[default]
    None,
    Add(String),
    Remove(String),
}

impl FlakeEdit {
    pub fn new(changes: Vec<Change>, walker: Walker) -> Self {
        Self { changes, walker }
    }

    pub fn from_text(stream: &str) -> Result<Self, FlakeEditError> {
        let walker = Walker::new(stream);
        Ok(Self::new(Vec::new(), walker))
    }

    pub fn changes(&self) -> &[Change] {
        self.changes.as_ref()
    }

    pub fn add_change(&mut self, change: Change) {
        self.changes.push(change);
    }

    pub fn curr_list(&self) -> &InputMap {
        &self.walker.inputs
    }

    /// Will walk and then list the inputs, for listing the current inputs,
    /// use `curr_list()`.
    pub fn list(&mut self) -> &InputMap {
        self.walker.inputs.clear();
        // Walk returns Ok(None) when no changes are made (expected for listing)
        assert!(self.walker.walk(&Change::None).ok().flatten().is_none());
        &self.walker.inputs
    }
    /// Apply a specific change to a walker, on some inputs it will need to walk
    /// multiple times, will error, if the edit could not be applied successfully.
    pub fn apply_change(&mut self, change: Change) -> Result<Option<String>, FlakeEditError> {
        match change {
            Change::None => Ok(None),
            Change::Add { .. } => {
                // Check for duplicate input before adding
                if let Some(input_id) = change.id() {
                    // First walk to populate the inputs map if it's empty
                    if self.walker.inputs.is_empty() {
                        let _ = self.walker.walk(&Change::None)?;
                    }

                    let input_id_string = input_id.to_string();
                    if self.walker.inputs.contains_key(&input_id_string) {
                        return Err(FlakeEditError::DuplicateInput(input_id_string));
                    }
                }

                if let Some(maybe_changed_node) = self.walker.walk(&change.clone())? {
                    let outputs = self.walker.list_outputs()?;
                    match outputs {
                        Outputs::Multiple(out) => {
                            let id = change.id().unwrap().to_string();
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
                // Ensure inputs are populated first so we can find orphaned follows
                if self.walker.inputs.is_empty() {
                    let _ = self.walker.walk(&Change::None)?;
                }

                let removed_id = change.id().unwrap().to_string();

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
                // Removed nodes should be removed from the outputs
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

                // Remove orphaned follows references that point to the removed input
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

                Ok(res.map(|n| n.to_string()))
            }
            Change::Pin { .. } => todo!(),
            Change::Change { .. } => {
                if let Some(input_id) = change.id() {
                    if self.walker.inputs.is_empty() {
                        let _ = self.walker.walk(&Change::None)?;
                    }

                    let input_id_string = input_id.to_string();
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

    /// Collect follows references that point to a removed input.
    /// Returns a list of Change::Remove for orphaned follows.
    fn collect_orphaned_follows(&self, removed_id: &str) -> Vec<Change> {
        let mut orphaned = Vec::new();
        for (input_id, input) in &self.walker.inputs {
            for follows in input.follows() {
                if let Follows::Indirect(follows_name, target) = follows {
                    // target is the RHS of `follows = "target"`
                    if target.trim_matches('"') == removed_id {
                        let nested_id = format!("{}.{}", input_id, follows_name);
                        orphaned.push(Change::Remove {
                            ids: vec![nested_id.into()],
                        });
                    }
                }
            }
        }
        orphaned
    }
}
