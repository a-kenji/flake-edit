/// A helper for the [`Walker`], in order to hold context while traversing the tree.
#[derive(Debug, Clone)]
pub struct Context {
    level: Vec<String>,
}

impl Context {
    /// Returns the first (top) level of context, if any.
    pub fn first(&self) -> Option<&str> {
        self.level.first().map(|s| s.as_str())
    }

    /// Returns true if the first level matches the given string.
    pub fn first_matches(&self, s: &str) -> bool {
        self.first() == Some(s)
    }

    pub fn level(&self) -> &[String] {
        &self.level
    }
}

impl From<String> for Context {
    fn from(s: String) -> Self {
        Self { level: vec![s] }
    }
}
