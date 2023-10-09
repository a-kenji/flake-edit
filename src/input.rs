use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Hash, Eq, Deserialize, Serialize, PartialOrd, Ord)]
pub struct Input {
    pub(crate) id: String,
    pub(crate) flake: bool,
    pub(crate) url: String,
    pub(crate) follows: Vec<Follows>,
}

#[derive(Debug, Clone, PartialEq, Hash, Eq, Deserialize, Serialize, PartialOrd, Ord)]
pub enum Follows {
    // From , To
    Indirect(String, String),
    // From , To
    Direct(String, Input),
}

impl Follows {}

#[derive(Debug, Default)]
pub(crate) struct FollowsBuilder {
    attrs: Vec<String>,
}

impl FollowsBuilder {
    pub(crate) fn push_str(&mut self, attr: &str) -> Option<Follows> {
        self.attrs.push(attr.to_owned());
        if self.attrs.len() == 4 {
            Some(self.build())
        } else {
            None
        }
    }
    fn build(&self) -> Follows {
        let from = self.attrs.get(1).unwrap();
        let to = self.attrs.get(3).unwrap();
        Follows::Indirect(from.to_owned(), to.to_owned())
    }
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

    pub fn id(&self) -> &str {
        self.id.as_ref()
    }
    pub fn url(&self) -> &str {
        self.url.as_ref()
    }
    pub fn follows(&self) -> &Vec<Follows> {
        self.follows.as_ref()
    }
}
