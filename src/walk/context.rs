use crate::follows::Segment;

/// Path of enclosing input identifiers tracked by [`super::Walker`] during CST traversal.
#[derive(Debug, Clone)]
pub(crate) struct Context {
    level: Vec<Segment>,
}

impl Context {
    /// Top-level enclosing segment, if any.
    pub(crate) fn first(&self) -> Option<&Segment> {
        self.level.first()
    }
}

impl From<Segment> for Context {
    fn from(s: Segment) -> Self {
        Self { level: vec![s] }
    }
}
