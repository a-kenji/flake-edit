use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Hash, Eq, Deserialize, Serialize)]
pub struct Input {
    pub id: String,
    pub flake: bool,
    pub url: String,
    follows: Vec<Follows>,
}

#[derive(Debug, Clone, PartialEq, Hash, Eq, Deserialize, Serialize)]
pub enum Follows {
    // From , To
    Indirect(String, String),
    // From , To
    Direct(String, Input),
}

impl Default for Input {
    fn default() -> Self {
        Self {
            id: String::new(),
            flake: true,
            url: String::new(),
            follows: vec![],
        }
    }
}

impl Input {
    pub(crate) fn new(name: String) -> Self {
        Self {
            id: name,
            ..Self::default()
        }
    }
}
