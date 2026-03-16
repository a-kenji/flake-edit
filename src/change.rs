use crate::walk::Context;

/// Split an input path at the first `.` that is outside double quotes.
///
/// Nix attributes containing dots must be quoted (e.g. `"hls-1.10"`).
/// A naive `split_once('.')` would split inside the quotes, so we skip
/// any dot that appears between an opening and closing `"`.
///
/// Examples:
///   `"hls-1.10".nixpkgs` -> `("hls-1.10", "nixpkgs")`
///   `crane.nixpkgs`      -> `("crane", "nixpkgs")`
///   `"hls-1.10"`         -> `None`
///   `nixpkgs`            -> `None`
pub fn split_quoted_path(s: &str) -> Option<(&str, &str)> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            // Skip to closing quote
            i += 1;
            while i < bytes.len() && bytes[i] != b'"' {
                i += 1;
            }
            // Skip the closing quote itself
            if i < bytes.len() {
                i += 1;
            }
        } else if bytes[i] == b'.' {
            return Some((&s[..i], &s[i + 1..]));
        } else {
            i += 1;
        }
    }
    None
}

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
        ref_or_rev: Option<String>,
    },
    /// Add a follows relationship to an input.
    /// Example: `flake-edit follow rust-overlay.nixpkgs nixpkgs`
    /// Creates: `rust-overlay.inputs.nixpkgs.follows = "nixpkgs";`
    Follows {
        /// The input path (e.g., "rust-overlay.nixpkgs" for rust-overlay's nixpkgs input)
        input: ChangeId,
        /// The target input to follow (e.g., "nixpkgs")
        target: String,
    },
}

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ChangeId(String);

impl ChangeId {
    /// Get the part after the dot (e.g., "nixpkgs" from "poetry2nix.nixpkgs").
    pub fn follows(&self) -> Option<&str> {
        split_quoted_path(&self.0).map(|(_, post)| post)
    }

    /// Get the input part (before the dot, or the whole thing if no dot).
    pub fn input(&self) -> &str {
        split_quoted_path(&self.0).map_or(&self.0, |(pre, _)| pre)
    }

    /// Check if this ChangeId matches the given input and optional follows.
    fn matches(&self, input: &str, follows: Option<&str>) -> bool {
        if self.input() != input {
            return false;
        }
        match (self.follows(), follows) {
            (Some(self_follows), Some(f)) => self_follows == f,
            (Some(_), None) => false,
            (None, _) => true,
        }
    }

    pub fn matches_with_follows(&self, input: &str, follows: Option<String>) -> bool {
        self.matches(input, follows.as_deref())
    }

    /// Match against context. The context carries the input attribute.
    pub fn matches_with_ctx(&self, follows: &str, ctx: Option<Context>) -> bool {
        let ctx_input = ctx.and_then(|f| f.level().first().cloned());
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
            Change::Remove { ids } => ids.first().cloned(),
            Change::Change { id, .. } => id.clone().map(|id| id.into()),
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
    pub fn is_some(&self) -> bool {
        !matches!(self, Change::None)
    }
    pub fn is_add(&self) -> bool {
        matches!(self, Change::Add { .. })
    }
    pub fn is_change(&self) -> bool {
        matches!(self, Change::Change { .. })
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
    pub fn follows_target(&self) -> Option<&String> {
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
                vec![format!(
                    "Added follows: {}.inputs.{}.follows = \"{}\"",
                    input.input(),
                    input.follows().unwrap_or("?"),
                    target
                )]
            }
            Change::None => vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_plain() {
        assert_eq!(
            split_quoted_path("crane.nixpkgs"),
            Some(("crane", "nixpkgs"))
        );
        assert_eq!(split_quoted_path("nixpkgs"), None);
    }

    #[test]
    fn split_quoted_dot_in_name() {
        assert_eq!(
            split_quoted_path("\"hls-1.10\".nixpkgs"),
            Some(("\"hls-1.10\"", "nixpkgs"))
        );
        assert_eq!(split_quoted_path("\"hls-1.10\""), None);
    }

    #[test]
    fn change_id_quoted_dot() {
        let id = ChangeId::from("\"hls-1.10\".nixpkgs".to_string());
        assert_eq!(id.input(), "\"hls-1.10\"");
        assert_eq!(id.follows(), Some("nixpkgs"));
    }
}
