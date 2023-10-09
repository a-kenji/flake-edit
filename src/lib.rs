pub mod diff;
pub mod error;
mod git;
pub mod input;
pub mod walk;

use std::collections::HashMap;

use self::input::Input;

#[derive(Debug, Default, Clone)]
pub struct State {
    // All the parsed inputs that are present in the attr set
    pub inputs: HashMap<String, Input>,
    changes: Vec<Change>,
}

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub enum Change {
    #[default]
    None,
    Add {
        id: Option<String>,
        uri: Option<String>,
    },
    Remove {
        id: String,
    },
    Pin {
        id: String,
    },
    Change {
        id: Option<String>,
        ref_or_rev: Option<String>,
    },
}

impl Change {
    pub fn id(&self) -> Option<String> {
        match self {
            Change::None => None,
            Change::Add { id, .. } => id.clone(),
            Change::Remove { id } => Some(id.clone()),
            Change::Change { id, .. } => id.clone(),
            Change::Pin { id } => Some(id.clone()),
        }
    }
    pub fn is_remove(&self) -> bool {
        matches!(self, Change::Remove { .. })
    }
}

impl State {
    pub fn add_change(&mut self, change: Change) {
        self.changes.push(change);
    }
    fn find_change(&self, target_id: String) -> Option<Change> {
        for change in &self.changes {
            match change {
                Change::None => {}
                Change::Remove { id } | Change::Pin { id } => {
                    if *id == target_id {
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
