#![allow(clippy::option_map_unit_fn)]
use crate::walk::Context;

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub enum Change {
    #[default]
    None,
    Add {
        id: Option<String>,
        uri: Option<String>,
        // Add an input as a flake.
        flake: bool,
    },
    Remove {
        id: ChangeId,
    },
    Pin {
        id: String,
    },
    Change {
        id: Option<String>,
        uri: Option<String>,
        ref_or_rev: Option<String>,
    },
}

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ChangeId(String);

impl ChangeId {
    pub fn follows(&self) -> Option<String> {
        let id = &self.0;
        let follows = id.split_once('.');
        follows.map(|(_pre, post)| post.into())
    }
    pub fn input(&self) -> Option<String> {
        let id = &self.0;
        let follows = id.split_once('.');
        if let Some((pre, _post)) = follows {
            Some(pre.into())
        } else {
            Some(id.clone())
        }
    }
    pub fn matches_with_follows(&self, input: &str, follows: Option<String>) -> bool {
        if let Some(input_id) = self.input() {
            if self.follows().is_some() {
                (self.follows() == follows) && (input_id == input)
            } else {
                input_id == input
            }
        } else {
            false
        }
    }
    // The context carries the input attribute
    pub fn matches_with_ctx(&self, follows: &str, ctx: Option<Context>) -> bool {
        let ctx = ctx.and_then(|f| f.level().first().cloned());

        if let Some(input_id) = self.input() {
            if let Some(ctx) = ctx {
                if self.follows().is_some() {
                    (input_id == ctx) && (self.follows() == Some(follows.into()))
                } else {
                    input_id == ctx
                }
            } else {
                input_id == follows
            }
        } else {
            false
        }
    }
}

impl std::fmt::Display for ChangeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for ChangeId {
    fn from(value: String) -> Self {
        ChangeId(value)
    }
}

impl Change {
    pub fn id(&self) -> Option<ChangeId> {
        match self {
            Change::None => None,
            Change::Add { id, .. } => id.clone().map(|id| id.into()),
            Change::Remove { id } => Some(id.clone()),
            Change::Change { id, .. } => id.clone().map(|id| id.into()),
            Change::Pin { id } => Some(id.clone().into()),
        }
    }
    pub fn is_remove(&self) -> bool {
        matches!(self, Change::Remove { .. })
    }
    pub fn is_some(&self) -> bool {
        !matches!(self, Change::None)
    }
    pub fn is_add(&self) -> bool {
        matches!(self, Change::Add { .. })
    }
    pub fn is_change(&self) -> bool {
        matches!(self, Change::Change { .. })
    }
    pub fn uri(&self) -> Option<&String> {
        match self {
            Change::Change { uri, .. } | Change::Add { uri, .. } => uri.as_ref(),
            _ => None,
        }
    }
}
