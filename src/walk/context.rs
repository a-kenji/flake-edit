use crate::follows::Segment;

/// A helper for the [`Walker`], in order to hold context while traversing the tree.
#[derive(Debug, Clone)]
pub struct Context {
    level: Vec<Segment>,
}

impl Context {
    /// Returns the first (top) level of context, if any.
    pub fn first(&self) -> Option<&Segment> {
        self.level.first()
    }

    /// Returns true if the first level matches the given segment.
    pub fn first_matches(&self, s: &Segment) -> bool {
        self.first() == Some(s)
    }

    pub fn level(&self) -> &[Segment] {
        &self.level
    }
}

impl From<Segment> for Context {
    fn from(s: Segment) -> Self {
        Self { level: vec![s] }
    }
}
