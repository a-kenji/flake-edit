use rnix::TextRange;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Hash, Eq, Deserialize, Serialize, PartialOrd, Ord)]
pub struct Input {
    pub(crate) id: String,
    pub(crate) flake: bool,
    pub(crate) url: String,
    pub(crate) follows: Vec<Follows>,
    pub range: Range,
}

#[derive(Debug, Default, Clone, PartialEq, Hash, Eq, Deserialize, Serialize, PartialOrd, Ord)]
pub struct Range {
    pub start: usize,
    pub end: usize,
}

impl Range {
    pub fn from_text_range(text_range: TextRange) -> Self {
        Self {
            start: text_range.start().into(),
            end: text_range.end().into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Hash, Eq, Deserialize, Serialize, PartialOrd, Ord)]
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
            range: Range::default(),
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

    /// Create an Input with id, url, and range set from a TextRange.
    pub(crate) fn with_url(id: String, url: String, text_range: TextRange) -> Self {
        Self {
            id,
            url,
            range: Range::from_text_range(text_range),
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
