use rnix::TextRange;

use crate::follows::{AttrPath, Segment};

#[derive(Debug, Clone, PartialEq, Hash, Eq, PartialOrd, Ord)]
pub struct Input {
    pub(crate) id: Segment,
    pub(crate) flake: bool,
    /// Always stored unquoted; rendering re-applies quoting via
    /// `make_quoted_string` at write-back time.
    pub(crate) url: String,
    pub(crate) follows: Vec<Follows>,
    pub range: Range,
}

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

    pub fn is_empty(&self) -> bool {
        self.start == 0 && self.end == 0
    }
}

#[derive(Debug, Clone, PartialEq, Hash, Eq, PartialOrd, Ord)]
pub enum Follows {
    /// A nested input (at any depth) redirected to another input.
    ///
    /// `path` is the nested-input chain relative to the owning [`Input`]:
    /// it does not include the owner's id segment. Examples:
    ///
    /// - `inputs.crane.inputs.nixpkgs.follows = "nixpkgs"` is stored on the
    ///   `crane` input as `Indirect { path: ["nixpkgs"], target:
    ///   AttrPath::parse("nixpkgs") }`.
    /// - `inputs.neovim.inputs.nixvim.inputs.flake-parts.follows =
    ///   "flake-parts"` is stored on `neovim` as `Indirect { path:
    ///   ["nixvim", "flake-parts"], target: AttrPath::parse("flake-parts") }`.
    Indirect { path: AttrPath, target: AttrPath },
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

    /// Create an Input with id, url, and range set from a TextRange.
    /// `url` is stored unquoted; if a quoted source token is passed, the
    /// surrounding double-quotes are stripped here.
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

    pub fn has_editable_url(&self) -> bool {
        !self.url.is_empty() && !self.range.is_empty()
    }
}

fn strip_outer_quotes(s: &str) -> &str {
    s.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(s)
}
