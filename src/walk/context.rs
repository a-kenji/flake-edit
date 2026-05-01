use crate::follows::Segment;

/// Path of enclosing input identifiers tracked by [`super::Walker`] during CST traversal.
#[derive(Debug, Clone)]
pub struct Context {
    level: Vec<Segment>,
}

impl Context {
    /// Top-level enclosing segment, if any.
    pub fn first(&self) -> Option<&Segment> {
        self.level.first()
    }

    /// Whether the top-level segment equals `s`.
    pub fn first_matches(&self, s: &Segment) -> bool {
        self.first() == Some(s)
    }
}

impl From<Segment> for Context {
    fn from(s: Segment) -> Self {
        Self { level: vec![s] }
    }
}
