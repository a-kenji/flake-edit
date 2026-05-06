use rnix::TextRange;

use crate::follows::{AttrPath, Segment, strip_outer_quotes};

/// A single flake input declaration.
#[derive(Debug, Clone, PartialEq, Hash, Eq, PartialOrd, Ord)]
pub struct Input {
    pub(crate) id: Segment,
    pub(crate) flake: bool,
    /// Stored unquoted. Quoting is re-applied at write-back time.
    pub(crate) url: String,
    pub(crate) follows: Vec<Follows>,
    pub range: Range,
}

/// Source byte range, half-open: `[start, end)`.
#[derive(Debug, Default, Clone, PartialEq, Hash, Eq, PartialOrd, Ord)]
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

    /// True if the range is the default (zero) range, used as a sentinel for
    /// inputs without a write-back location.
    pub fn is_empty(&self) -> bool {
        self.start == 0 && self.end == 0
    }
}

/// A `follows` declaration on an [`Input`].
#[derive(Debug, Clone, PartialEq, Hash, Eq, PartialOrd, Ord)]
pub enum Follows {
    /// A nested input redirected to another input via `follows = "..."`.
    ///
    /// `path` is the nested-input chain relative to the owning [`Input`] and
    /// does not include the owner's id segment. `target` is the right-hand
    /// side of the `follows = "..."`; `None` represents the empty-string
    /// form `follows = ""`, the in-flake equivalent of the lockfile's
    /// [`crate::lock::Input::Indirect`]`(None)` (an `inputs.X = []` entry).
    ///
    /// - `inputs.crane.inputs.nixpkgs.follows = "nixpkgs"` is stored on
    ///   `crane` as `Indirect { path: ["nixpkgs"], target: Some(["nixpkgs"]) }`.
    /// - `inputs.neovim.inputs.nixvim.inputs.flake-parts.follows =
    ///   "flake-parts"` is stored on `neovim` as `Indirect { path:
    ///   ["nixvim", "flake-parts"], target: Some(["flake-parts"]) }`.
    /// - `inputs.nix.inputs.flake-compat.follows = ""` is stored on `nix`
    ///   as `Indirect { path: ["flake-compat"], target: None }`.
    Indirect {
        path: AttrPath,
        target: Option<AttrPath>,
    },
    /// A nested input declared inline with its own URL.
    Direct(String, Input),
}

impl Input {
    pub(crate) fn new(name: Segment) -> Self {
        Self {
            id: name,
            flake: true,
            url: String::new(),
            follows: Vec::new(),
            range: Range::default(),
        }
    }

    /// Build an [`Input`] with `id`, `url`, and the range derived from
    /// `text_range`. Surrounding double-quotes on `url` are stripped.
    pub(crate) fn with_url(id: Segment, url: String, text_range: TextRange) -> Self {
        Self {
            id,
            flake: true,
            url: strip_outer_quotes(&url).to_string(),
            follows: Vec::new(),
            range: Range::from_text_range(text_range),
        }
    }

    pub fn id(&self) -> &Segment {
        &self.id
    }

    pub fn url(&self) -> &str {
        self.url.as_ref()
    }
    pub fn follows(&self) -> &Vec<Follows> {
        self.follows.as_ref()
    }

    /// True if the URL can be rewritten in place. False for synthetic inputs
    /// without a known source range.
    pub fn has_editable_url(&self) -> bool {
        !self.url.is_empty() && !self.range.is_empty()
    }

    /// Append an `Indirect` follows entry and re-normalize the follows vec
    /// (sort + dedup). Walker insertion sites maintain this invariant so
    /// callers downstream (validate, follows-graph, snapshots) see one
    /// canonical ordering.
    pub(crate) fn push_indirect_follows(&mut self, path: AttrPath, target: Option<AttrPath>) {
        self.follows.push(Follows::Indirect { path, target });
        self.follows.sort();
        self.follows.dedup();
    }
}
