use crate::follows::{AttrPath, AttrPathParseError, Segment};
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
        ids: Vec<ChangeId>,
    },
    Change {
        id: Option<String>,
        uri: Option<String>,
    },
    /// Redirect a nested input to follow another input.
    ///
    /// Applying `Follows { input: "rust-overlay.nixpkgs", target: "nixpkgs" }`
    /// writes `rust-overlay.inputs.nixpkgs.follows = "nixpkgs";`.
    Follows {
        /// Path to the nested input being redirected (e.g.
        /// `rust-overlay.nixpkgs`).
        input: ChangeId,
        /// The input to follow.
        target: AttrPath,
    },
}

/// Identifier for an input or nested-input target of a [`Change`].
///
/// Wraps an [`AttrPath`]: a non-empty sequence of unquoted segments matching
/// flake-side attribute path grammar.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ChangeId(AttrPath);

impl ChangeId {
    pub fn new(path: AttrPath) -> Self {
        ChangeId(path)
    }

    /// Parse a dotted attribute path. Quoted segments (e.g. `"hls-1.10"`)
    /// retain dots that would otherwise act as separators.
    pub fn parse(s: &str) -> Result<Self, AttrPathParseError> {
        Ok(ChangeId(AttrPath::parse(s)?))
    }

    pub fn path(&self) -> &AttrPath {
        &self.0
    }

    /// Top-level input segment: the segment before the first dot, or the whole
    /// path if it has only one segment.
    pub fn input(&self) -> &Segment {
        self.0.first()
    }

    /// Second segment, if the path has more than one segment.
    pub fn follows(&self) -> Option<&Segment> {
        self.0.child()
    }

    fn matches(&self, input: &Segment, follows: Option<&Segment>) -> bool {
        if self.input() != input {
            return false;
        }
        match (self.follows(), follows) {
            (Some(self_follows), Some(f)) => self_follows == f,
            (Some(_), None) => false,
            (None, _) => true,
        }
    }

    /// Match against an explicit `(input, follows)` pair.
    pub fn matches_with_follows(&self, input: &Segment, follows: Option<&Segment>) -> bool {
        self.matches(input, follows)
    }

    /// Match against the surrounding walker [`Context`], which carries the
    /// enclosing top-level input.
    pub fn matches_with_ctx(&self, follows: &Segment, ctx: Option<Context>) -> bool {
        let ctx_input = ctx.and_then(|c| c.first().cloned());
        match ctx_input {
            Some(input) => self.matches(&input, Some(follows)),
            None => self.input() == follows,
        }
    }
}

impl std::fmt::Display for ChangeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TryFrom<String> for ChangeId {
    type Error = AttrPathParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        ChangeId::parse(&value)
    }
}

impl TryFrom<&str> for ChangeId {
    type Error = AttrPathParseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        ChangeId::parse(value)
    }
}

impl From<AttrPath> for ChangeId {
    fn from(value: AttrPath) -> Self {
        ChangeId(value)
    }
}

impl From<Segment> for ChangeId {
    fn from(value: Segment) -> Self {
        ChangeId(AttrPath::new(value))
    }
}

impl Change {
    pub fn id(&self) -> Option<ChangeId> {
        match self {
            Change::None => None,
            Change::Add { id, .. } => id.clone().and_then(|id| ChangeId::parse(&id).ok()),
            Change::Remove { ids } => ids.first().cloned(),
            Change::Change { id, .. } => id.clone().and_then(|id| ChangeId::parse(&id).ok()),
            Change::Follows { input, .. } => Some(input.clone()),
        }
    }

    pub fn ids(&self) -> Vec<ChangeId> {
        match self {
            Change::Remove { ids } => ids.clone(),
            Change::Follows { input, .. } => vec![input.clone()],
            _ => self.id().into_iter().collect(),
        }
    }
    pub fn is_remove(&self) -> bool {
        matches!(self, Change::Remove { .. })
    }
    pub fn is_follows(&self) -> bool {
        matches!(self, Change::Follows { .. })
    }
    pub fn uri(&self) -> Option<&String> {
        match self {
            Change::Change { uri, .. } | Change::Add { uri, .. } => uri.as_ref(),
            _ => None,
        }
    }
    pub fn follows_target(&self) -> Option<&AttrPath> {
        match self {
            Change::Follows { target, .. } => Some(target),
            _ => None,
        }
    }

    pub fn success_messages(&self) -> Vec<String> {
        match self {
            Change::Add { id, uri, .. } => {
                vec![format!(
                    "Added input: {} = {}",
                    id.as_deref().unwrap_or("?"),
                    uri.as_deref().unwrap_or("?")
                )]
            }
            Change::Remove { ids } => ids
                .iter()
                .map(|id| format!("Removed input: {}", id))
                .collect(),
            Change::Change { id, uri, .. } => {
                vec![format!(
                    "Changed input: {} -> {}",
                    id.as_deref().unwrap_or("?"),
                    uri.as_deref().unwrap_or("?")
                )]
            }
            Change::Follows { input, target } => {
                // Interleave segments with `.inputs.`: `[a, b, c]` renders as
                // `a.inputs.b.inputs.c`. Length-1 paths get an `inputs.` prefix.
                let segments = input.path().segments();
                let path = if segments.len() == 1 {
                    format!("inputs.{}", segments[0].render())
                } else {
                    let mut out = String::new();
                    for (i, seg) in segments.iter().enumerate() {
                        if i == 0 {
                            out.push_str(&seg.render());
                        } else {
                            out.push_str(".inputs.");
                            out.push_str(&seg.render());
                        }
                    }
                    out
                };
                vec![format!("Added follows: {}.follows = \"{}\"", path, target)]
            }
            Change::None => vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_id_quoted_dot() {
        let id = ChangeId::parse("\"hls-1.10\".nixpkgs").unwrap();
        assert_eq!(id.input().as_str(), "hls-1.10");
        assert_eq!(id.follows().unwrap().as_str(), "nixpkgs");
    }

    #[test]
    fn change_id_single_segment_no_follows() {
        let id = ChangeId::parse("nixpkgs").unwrap();
        assert_eq!(id.input().as_str(), "nixpkgs");
        assert!(id.follows().is_none());
    }

    #[test]
    fn success_message_depth_three_has_two_inputs_separators() {
        let change = Change::Follows {
            input: ChangeId::parse("neovim.nixvim.flake-parts").unwrap(),
            target: AttrPath::parse("flake-parts").unwrap(),
        };
        let msgs = change.success_messages();
        assert_eq!(msgs.len(), 1);
        let msg = &msgs[0];
        let inputs_count = msg.matches(".inputs.").count();
        assert_eq!(
            inputs_count, 2,
            "depth-3 message should contain exactly two `.inputs.` separators, got: {msg}"
        );
    }

    #[test]
    fn success_message_depth_two_has_one_inputs_separator() {
        let change = Change::Follows {
            input: ChangeId::parse("crane.nixpkgs").unwrap(),
            target: AttrPath::parse("nixpkgs").unwrap(),
        };
        let msgs = change.success_messages();
        let msg = &msgs[0];
        assert_eq!(msg.matches(".inputs.").count(), 1);
    }

    #[test]
    fn success_message_depth_one_uses_inputs_prefix() {
        let change = Change::Follows {
            input: ChangeId::parse("nixpkgs").unwrap(),
            target: AttrPath::parse("foo").unwrap(),
        };
        let msgs = change.success_messages();
        let msg = &msgs[0];
        assert!(
            msg.starts_with("Added follows: inputs.nixpkgs.follows ="),
            "depth-1 message should start with `inputs.<id>.follows =`, got: {msg}"
        );
    }
}
