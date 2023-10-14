use crate::walk::Context;

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub enum Change {
    #[default]
    None,
    Add {
        id: Option<String>,
        uri: Option<String>,
    },
    Remove {
        id: ChangeId,
    },
    Pin {
        id: String,
    },
    Change {
        id: Option<String>,
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
        follows.map(|(pre, _post)| pre.into())
    }
    pub fn matches_with_follows(&self, input: &str, follows: Option<&str>) -> bool {
        let follows = follows.map(|f| f.to_string());

        if let Some(input_id) = self.input() {
            (self.follows() == follows) && (input_id == input)
        } else {
            false
        }
    }
    pub fn matches_with_ctx(&self, input: &str, follows: Option<Context>) -> bool {
        let follows = follows.and_then(|f| f.level().first().cloned());

        if let Some(input_id) = self.input() {
            (self.follows() == follows) && (input_id == input)
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
            Change::Remove { id } => Some(id.clone().into()),
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
}
