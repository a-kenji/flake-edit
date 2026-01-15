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
    Pin {
        id: String,
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
        self.0.split_once('.').map(|(_, post)| post)
    }

    /// Get the input part (before the dot, or the whole thing if no dot).
    pub fn input(&self) -> &str {
        self.0.split_once('.').map_or(&self.0, |(pre, _)| pre)
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
            Change::Pin { id } => Some(id.clone().into()),
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
}
