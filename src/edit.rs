use std::collections::HashMap;

use crate::change::Change;
use crate::error::FlakeEditError;
use crate::input::Input;
use crate::walk::Walker;

pub struct FlakeEdit {
    changes: Vec<Change>,
    walker: Walker,
}

#[derive(Default, Debug)]
pub enum Outputs {
    #[default]
    None,
    // needs strict inputs to output mapping
    Multiple(Vec<String>),
    // contains ...
    Any(Vec<String>),
}

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

    pub fn from(stream: &str) -> Result<Self, FlakeEditError> {
        let walker = Walker::new(stream);
        Ok(Self::new(Vec::new(), walker))
    }

    pub fn changes(&self) -> &[Change] {
        self.changes.as_ref()
    }

    pub fn add_change(&mut self, change: Change) {
        self.changes.push(change);
    }

    pub fn curr_list(&self) -> &HashMap<String, Input> {
        &self.walker.inputs
    }

    /// Will walk and then list the inputs, for listing the current inputs,
    /// use `curr_list()`.
    pub fn list(&mut self) -> &HashMap<String, Input> {
        self.walker.inputs.clear();
        assert!(self.walker.walk(&Change::None).is_none());
        &self.walker.inputs
    }
    /// Apply a specific change to a walker, on some inputs it will need to walk
    /// multiple times, will error, if the edit could not be applied successfully.
    pub fn apply_change(&mut self, change: Change) -> Result<Option<String>, FlakeEditError> {
        match change {
            Change::None => Ok(None),
            //TODO: Add outputs, if needed.
            Change::Add { .. } => {
                if let Some(maybe_changed_node) = self.walker.walk(&change.clone()) {
                    let outputs = self.walker.list_outputs();
                    match outputs {
                        Outputs::Multiple(out) => {
                            let id = change.id().unwrap().to_string();
                            if !out.contains(&id) {
                                self.walker.root = maybe_changed_node.clone();
                                if let Some(maybe_changed_node) =
                                    self.walker.change_outputs(OutputChange::Add(id))
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
                    let maybe_changed_node = self.walker.walk(&change);
                    Ok(maybe_changed_node.map(|n| n.to_string()))
                }
            }
            Change::Remove { .. } => {
                // If we remove a node, it could be a flat structure,
                // we want to remove all of the references to its toplevel.
                let mut res = None;
                while let Some(changed_node) = self.walker.walk(&change) {
                    if res == Some(changed_node.clone()) {
                        // TODO: Sanity check, turn into proper error.
                        break;
                    }
                    res = Some(changed_node.clone());
                    self.walker.root = changed_node.clone();
                }
                // Removed nodes should be removed from the outputs
                let outputs = self.walker.list_outputs();
                match outputs {
                    Outputs::Multiple(out) => {
                        let id = change.id().unwrap().to_string();
                        if out.contains(&id) {
                            if let Some(changed_node) =
                                self.walker.change_outputs(OutputChange::Remove(id))
                            {
                                res = Some(changed_node.clone());
                                self.walker.root = changed_node.clone();
                            }
                        }
                    }
                    Outputs::None | Outputs::Any(_) => {}
                }
                Ok(res.map(|n| n.to_string()))
            }
            Change::Pin { .. } => todo!(),
            Change::Change { .. } => {
                if let Some(maybe_changed_node) = self.walker.walk(&change) {
                    Ok(Some(maybe_changed_node.to_string()))
                } else {
                    panic!("No change");
                }
            }
        }
    }

    pub fn walker(&self) -> &Walker {
        &self.walker
    }
}
