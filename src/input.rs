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

impl Follows {}

#[derive(Debug, Default)]
struct FollowsBuilder {
    attrs: Vec<String>,
}

impl FollowsBuilder {
    pub(crate) fn push_str(&mut self, attr: &str) -> Option<Follows> {
        self.attrs.push(attr.to_owned());
        if attr.len() == 3 {
            Some(self.build())
        } else {
            None
        }
    }
    fn build(&self) -> Follows {
        let from = self.attrs.get(0).unwrap();
        let to = self.attrs.get(1).unwrap();
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
}
