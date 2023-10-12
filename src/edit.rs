use std::collections::HashMap;

use crate::change::Change;
use crate::error::FlakeEditError;
use crate::input::Input;
use crate::walk::Walker;

pub struct FlakeEdit {
    changes: Vec<Change>,
    walker: Walker,
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
        assert!(self.walker.walk(&Change::None).is_none());
        &self.walker.inputs
    }

    /// Apply a specific change to a walker, on some inputs, it will need to walk
    /// multiple times, will error, if the edit could not be applied successfully.
    pub fn apply_change(&mut self, change: Change) -> Result<Option<String>, FlakeEditError> {
        let maybe_changed_node = self.walker.walk(&change);
        match change {
            Change::None => {
                assert!(maybe_changed_node.is_none())
            }
            Change::Add { id, uri } => {}
            Change::Remove { id } => {
                // If we remove a node, it could be a flat structure,
                // we want to remove all of the references to its toplevel.
                println!("{:#?}", self.curr_list());
            }
            Change::Pin { id } => todo!(),
            Change::Change { id, ref_or_rev } => todo!(),
        }
        Ok(maybe_changed_node.map(|n| n.to_string()))
    }
}
