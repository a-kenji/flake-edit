use std::collections::HashMap;

use crate::change::Change;
use crate::input;
use crate::input::Input;

#[derive(Debug, Default, Clone)]
pub struct State {
    // All the parsed inputs that are present in the attr set
    pub inputs: HashMap<String, Input>,
    changes: Vec<Change>,
}

impl State {
    pub fn add_change(&mut self, change: Change) {
        self.changes.push(change);
    }
    fn find_change(&self, target_id: String) -> Option<Change> {
        for change in &self.changes {
            match change {
                Change::None => {}
                Change::Pin { id: _ } => {
                    todo!()
                }
                Change::Remove { id } => {
                    if id.to_string() == target_id {
                        return Some(change.clone());
                    }
                }
                Change::Add { id, .. } | Change::Change { id, .. } => {
                    if let Some(id) = id {
                        if *id == target_id {
                            return Some(change.clone());
                        }
                    }
                }
            }
        }
        None
    }
    pub fn add_input(&mut self, key: &str, input: Input) {
        self.inputs.insert(key.into(), input);
    }
    pub fn add_follows(&mut self, key: &str, follows: input::Follows) {
        if let Some(input) = self.inputs.get_mut(key) {
            input.follows.push(follows);
        }
    }
}
