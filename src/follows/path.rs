//! [`Segment`] and [`AttrPath`]: typed attribute paths.
//!
//! [`Segment`] is a single attribute name. [`AttrPath`] is a non-empty
//! sequence of them. Both store values unquoted. The `"..."` quotes Nix
//! requires for names containing dots or leading digits live on the rendering
//! boundary ([`fmt::Display`] / [`Segment::render`]).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smallvec::{SmallVec, smallvec};

/// Strip a single outer pair of `"..."` from CST source text.
///
/// Returns the input unchanged when it is not bracketed by quotes.
pub fn strip_outer_quotes(s: &str) -> &str {
    s.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(s)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Segment(String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SegmentError {
    Empty,
    ContainsQuote,
    ContainsControl,
}

impl fmt::Display for SegmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SegmentError::Empty => write!(f, "segment must not be empty"),
            SegmentError::ContainsQuote => {
                write!(f, "segment must not contain an embedded double quote")
            }
            SegmentError::ContainsControl => {
                write!(f, "segment must not contain control characters")
            }
        }
    }
}

impl std::error::Error for SegmentError {}

impl Segment {
    /// Construct from already-unquoted text.
    ///
    /// Rejects empty input, embedded `"`, and ASCII control characters.
    /// Everything else (`.`, `+`, `/`, leading digits, hyphens, single quotes)
    /// is accepted. [`Self::render`] decides whether to wrap in `"..."`.
    pub fn from_unquoted(s: impl Into<String>) -> Result<Self, SegmentError> {
        let s = s.into();
        if s.is_empty() {
            return Err(SegmentError::Empty);
        }
        if s.contains('"') {
            return Err(SegmentError::ContainsQuote);
        }
        if s.chars().any(|c| c.is_control()) {
            return Err(SegmentError::ContainsControl);
        }
        Ok(Segment(s))
    }

    /// Parse source-form text. Strips a single surrounding pair of `"..."`,
    /// otherwise behaves like [`Self::from_unquoted`].
    pub fn from_source(s: &str) -> Result<Self, SegmentError> {
        let body = if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
            &s[1..s.len() - 1]
        } else {
            s
        };
        Segment::from_unquoted(body.to_string())
    }

    /// Build a [`Segment`] from a CST node's source text.
    pub fn from_syntax(node: &rnix::SyntaxNode) -> Result<Self, SegmentError> {
        Segment::from_source(&node.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume the segment, returning the unquoted text.
    pub fn into_string(self) -> String {
        self.0
    }

    /// Whether this segment requires source-level `"..."` quoting.
    ///
    /// Bare Nix identifiers match `[a-zA-Z_][a-zA-Z0-9_'-]*`. Anything else
    /// (leading digit, embedded `.`, leading `-`) needs quoting.
    pub fn needs_quoting(&self) -> bool {
        let mut chars = self.0.chars();
        let Some(first) = chars.next() else {
            return true;
        };
        if !(first.is_ascii_alphabetic() || first == '_') {
            return true;
        }
        for c in chars {
            if !(c.is_ascii_alphanumeric() || c == '_' || c == '\'' || c == '-') {
                return true;
            }
        }
        false
    }

    /// Render to source form, wrapping in `"..."` only when needed.
    pub fn render(&self) -> String {
        if self.needs_quoting() {
            format!("\"{}\"", self.0)
        } else {
            self.0.clone()
        }
    }
}

impl fmt::Display for Segment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render())
    }
}

impl FromStr for Segment {
    type Err = SegmentError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Segment::from_source(s)
    }
}

impl Serialize for Segment {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Segment {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Segment::from_unquoted(s).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AttrPath(SmallVec<[Segment; 2]>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttrPathParseError {
    Empty,
    EmptySegment,
    SegmentInvalid(SegmentError),
}

impl fmt::Display for AttrPathParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AttrPathParseError::Empty => write!(f, "attribute path must not be empty"),
            AttrPathParseError::EmptySegment => write!(f, "attribute path has an empty segment"),
            AttrPathParseError::SegmentInvalid(e) => write!(f, "invalid segment: {e}"),
        }
    }
}

impl std::error::Error for AttrPathParseError {}

impl From<SegmentError> for AttrPathParseError {
    fn from(value: SegmentError) -> Self {
        AttrPathParseError::SegmentInvalid(value)
    }
}

impl AttrPath {
    pub fn new(first: Segment) -> Self {
        AttrPath(smallvec![first])
    }

    /// Parse a dotted path, respecting `"..."` quoting on individual segments.
    ///
    /// Examples:
    /// - `nixpkgs` → 1 segment.
    /// - `crane.nixpkgs` → 2 segments.
    /// - `"hls-1.10".nixpkgs` → 2 segments, the first stored unquoted.
    /// - `a."b.c".d` → 3 segments.
    pub fn parse(s: &str) -> Result<Self, AttrPathParseError> {
        if s.is_empty() {
            return Err(AttrPathParseError::Empty);
        }
        let mut segments: SmallVec<[Segment; 2]> = SmallVec::new();
        let bytes = s.as_bytes();
        let mut start = 0;
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'"' {
                // Skip until matching closing quote.
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1; // skip closing quote
                }
            } else if bytes[i] == b'.' {
                let raw = &s[start..i];
                if raw.is_empty() {
                    return Err(AttrPathParseError::EmptySegment);
                }
                segments.push(Segment::from_source(raw)?);
                i += 1;
                start = i;
            } else {
                i += 1;
            }
        }
        let last = &s[start..];
        if last.is_empty() {
            return Err(AttrPathParseError::EmptySegment);
        }
        segments.push(Segment::from_source(last)?);
        Ok(AttrPath(segments))
    }

    pub fn first(&self) -> &Segment {
        &self.0[0]
    }

    pub fn last(&self) -> &Segment {
        self.0.last().expect("AttrPath is non-empty by invariant")
    }

    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn segments(&self) -> &[Segment] {
        &self.0
    }

    /// All segments except the last, or `None` for a length-1 path.
    pub fn parent(&self) -> Option<AttrPath> {
        if self.0.len() <= 1 {
            return None;
        }
        let parent_segments: SmallVec<[Segment; 2]> =
            self.0[..self.0.len() - 1].iter().cloned().collect();
        Some(AttrPath(parent_segments))
    }

    /// The second segment, or `None` for length-1 paths.
    pub fn child(&self) -> Option<&Segment> {
        if self.0.len() >= 2 {
            self.0.get(1)
        } else {
            None
        }
    }

    pub fn push(&mut self, seg: Segment) {
        self.0.push(seg);
    }

    /// Whether `self` is a structural prefix of `other`. A path is its own
    /// prefix.
    pub fn is_prefix_of(&self, other: &AttrPath) -> bool {
        if self.0.len() > other.0.len() {
            return false;
        }
        self.0.iter().zip(other.0.iter()).all(|(a, b)| a == b)
    }

    /// If `base` is a strict prefix of `self`, returns the suffix. Returns
    /// `None` otherwise, including when the paths are equal.
    pub fn relative_to(&self, base: &AttrPath) -> Option<AttrPath> {
        if !base.is_prefix_of(self) || base.0.len() == self.0.len() {
            return None;
        }
        let suffix: SmallVec<[Segment; 2]> = self.0[base.0.len()..].iter().cloned().collect();
        Some(AttrPath(suffix))
    }

    /// Render as the `parent/child/grandchild` form used by lockfile
    /// `follows = [...]` arrays: `/` separators, each segment unquoted.
    pub fn to_flake_follows_string(&self) -> String {
        self.0
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("/")
    }
}

impl fmt::Display for AttrPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for seg in &self.0 {
            if !first {
                f.write_str(".")?;
            }
            first = false;
            f.write_str(&seg.render())?;
        }
        Ok(())
    }
}

impl FromStr for AttrPath {
    type Err = AttrPathParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        AttrPath::parse(s)
    }
}

impl From<Segment> for AttrPath {
    fn from(value: Segment) -> Self {
        AttrPath::new(value)
    }
}

impl Serialize for AttrPath {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for AttrPath {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        AttrPath::parse(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_from_unquoted_rejects_empty() {
        assert_eq!(Segment::from_unquoted(""), Err(SegmentError::Empty));
    }

    #[test]
    fn segment_from_unquoted_rejects_embedded_quote() {
        assert_eq!(
            Segment::from_unquoted("a\"b"),
            Err(SegmentError::ContainsQuote)
        );
    }

    #[test]
    fn segment_from_unquoted_rejects_control() {
        assert_eq!(
            Segment::from_unquoted("a\nb"),
            Err(SegmentError::ContainsControl)
        );
    }

    #[test]
    fn segment_from_unquoted_accepts_dotted() {
        let s = Segment::from_unquoted("hls-1.10").unwrap();
        assert_eq!(s.as_str(), "hls-1.10");
    }

    #[test]
    fn segment_from_source_strips_quotes() {
        let s = Segment::from_source("\"hls-1.10\"").unwrap();
        assert_eq!(s.as_str(), "hls-1.10");
    }

    #[test]
    fn segment_from_source_unquoted_passthrough() {
        let s = Segment::from_source("nixpkgs").unwrap();
        assert_eq!(s.as_str(), "nixpkgs");
    }

    #[test]
    fn segment_from_syntax_via_rnix() {
        // Build a tiny CST and route the first NODE_STRING through
        // `from_syntax` to verify the round-trip.
        let src = r#"{ inputs."hls-1.10".url = "x"; }"#;
        let parsed = rnix::Root::parse(src);
        let syntax = parsed.syntax();
        fn find_string(node: rnix::SyntaxNode) -> Option<rnix::SyntaxNode> {
            if node.kind() == rnix::SyntaxKind::NODE_STRING {
                return Some(node);
            }
            for c in node.children() {
                if let Some(s) = find_string(c) {
                    return Some(s);
                }
            }
            None
        }
        let string_node = find_string(syntax).expect("has a string node");
        let seg = Segment::from_syntax(&string_node).unwrap();
        // The first NODE_STRING in source order is "hls-1.10".
        assert_eq!(seg.as_str(), "hls-1.10");
    }

    #[test]
    fn segment_needs_quoting_boundaries() {
        for bare in ["nixpkgs", "_x", "foo'bar"] {
            assert!(
                !Segment::from_unquoted(bare).unwrap().needs_quoting(),
                "{bare} should be a bare ident",
            );
        }
        for quoted in ["hls-1.10", "24.11", "-x"] {
            assert!(
                Segment::from_unquoted(quoted).unwrap().needs_quoting(),
                "{quoted} should require quoting",
            );
        }
    }

    #[test]
    fn segment_render_unquoted() {
        let s = Segment::from_unquoted("nixpkgs").unwrap();
        assert_eq!(s.render(), "nixpkgs");
    }

    #[test]
    fn segment_render_quoted() {
        let s = Segment::from_unquoted("hls-1.10").unwrap();
        assert_eq!(s.render(), "\"hls-1.10\"");
    }

    #[test]
    fn segment_display_matches_render() {
        let s = Segment::from_unquoted("hls-1.10").unwrap();
        assert_eq!(format!("{s}"), s.render());
    }

    #[test]
    fn segment_from_str_uses_from_source() {
        let s: Segment = "\"hls-1.10\"".parse().unwrap();
        assert_eq!(s.as_str(), "hls-1.10");
    }

    #[test]
    fn segment_serde_roundtrip_bare() {
        let s = Segment::from_unquoted("nixpkgs").unwrap();
        let j = serde_json::to_string(&s).unwrap();
        assert_eq!(j, "\"nixpkgs\"");
        let back: Segment = serde_json::from_str(&j).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn segment_serde_roundtrip_dotted() {
        let s = Segment::from_unquoted("hls-1.10").unwrap();
        let j = serde_json::to_string(&s).unwrap();
        // Wire form has no embedded backslash-quote.
        assert_eq!(j, "\"hls-1.10\"");
        let back: Segment = serde_json::from_str(&j).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn attr_path_parse_single_segment() {
        let p = AttrPath::parse("nixpkgs").unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p.first().as_str(), "nixpkgs");
    }

    #[test]
    fn attr_path_parse_two_segments() {
        let p = AttrPath::parse("crane.nixpkgs").unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(p.first().as_str(), "crane");
        assert_eq!(p.last().as_str(), "nixpkgs");
    }

    #[test]
    fn attr_path_parse_quoted_first() {
        let p = AttrPath::parse("\"hls-1.10\".nixpkgs").unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(p.first().as_str(), "hls-1.10");
        assert_eq!(p.last().as_str(), "nixpkgs");
    }

    #[test]
    fn attr_path_parse_three_segments_middle_quoted() {
        let p = AttrPath::parse("a.\"b.c\".d").unwrap();
        assert_eq!(p.len(), 3);
        assert_eq!(p.segments()[0].as_str(), "a");
        assert_eq!(p.segments()[1].as_str(), "b.c");
        assert_eq!(p.segments()[2].as_str(), "d");
    }

    #[test]
    fn attr_path_parse_empty_rejected() {
        assert_eq!(AttrPath::parse(""), Err(AttrPathParseError::Empty));
    }

    #[test]
    fn attr_path_parse_double_dot_rejected() {
        assert_eq!(
            AttrPath::parse("a..b"),
            Err(AttrPathParseError::EmptySegment)
        );
    }

    #[test]
    fn attr_path_display_roundtrip() {
        for s in ["crane.nixpkgs", "\"hls-1.10\".nixpkgs"] {
            let p = AttrPath::parse(s).unwrap();
            assert_eq!(format!("{p}"), s);
        }
    }

    #[test]
    fn attr_path_parent_none_for_single() {
        let p = AttrPath::parse("nixpkgs").unwrap();
        assert!(p.parent().is_none());
    }

    #[test]
    fn attr_path_parent_some_for_two() {
        let p = AttrPath::parse("crane.nixpkgs").unwrap();
        let parent = p.parent().unwrap();
        assert_eq!(parent.len(), 1);
        assert_eq!(parent.first().as_str(), "crane");
    }

    #[test]
    fn attr_path_child_returns_second_segment() {
        let p = AttrPath::parse("crane.nixpkgs").unwrap();
        assert_eq!(p.child().unwrap().as_str(), "nixpkgs");
    }

    #[test]
    fn attr_path_child_none_for_single() {
        let p = AttrPath::parse("crane").unwrap();
        assert!(p.child().is_none());
    }

    #[test]
    fn attr_path_push_extends() {
        let mut p = AttrPath::parse("a").unwrap();
        p.push(Segment::from_unquoted("b").unwrap());
        assert_eq!(format!("{p}"), "a.b");
    }

    #[test]
    fn attr_path_is_prefix_self() {
        let p = AttrPath::parse("a.b").unwrap();
        assert!(p.is_prefix_of(&p));
    }

    #[test]
    fn attr_path_is_prefix_strict() {
        let a = AttrPath::parse("a").unwrap();
        let ab = AttrPath::parse("a.b").unwrap();
        assert!(a.is_prefix_of(&ab));
        assert!(!ab.is_prefix_of(&a));
    }

    #[test]
    fn attr_path_is_prefix_diverging() {
        let a = AttrPath::parse("a.x").unwrap();
        let b = AttrPath::parse("a.y").unwrap();
        assert!(!a.is_prefix_of(&b));
    }

    #[test]
    fn attr_path_relative_to_strict_prefix() {
        let base = AttrPath::parse("a").unwrap();
        let path = AttrPath::parse("a.b.c").unwrap();
        let rel = path.relative_to(&base).unwrap();
        assert_eq!(format!("{rel}"), "b.c");
    }

    #[test]
    fn attr_path_relative_to_equal_returns_none() {
        let p = AttrPath::parse("a.b").unwrap();
        assert!(p.relative_to(&p).is_none());
    }

    #[test]
    fn attr_path_relative_to_non_prefix_returns_none() {
        let base = AttrPath::parse("c").unwrap();
        let path = AttrPath::parse("a.b").unwrap();
        assert!(path.relative_to(&base).is_none());
    }

    #[test]
    fn attr_path_from_segment() {
        let s = Segment::from_unquoted("nixpkgs").unwrap();
        let p: AttrPath = s.clone().into();
        assert_eq!(p.len(), 1);
        assert_eq!(p.first(), &s);
    }

    #[test]
    fn attr_path_from_str_parses() {
        let p: AttrPath = "crane.nixpkgs".parse().unwrap();
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn attr_path_serde_roundtrip() {
        let p = AttrPath::parse("\"hls-1.10\".nixpkgs").unwrap();
        let j = serde_json::to_string(&p).unwrap();
        // Wire form is the canonical Display output (quoted as needed).
        assert_eq!(j, "\"\\\"hls-1.10\\\".nixpkgs\"");
        let back: AttrPath = serde_json::from_str(&j).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn attr_path_to_flake_follows_string_simple() {
        let p = AttrPath::parse("nixpkgs").unwrap();
        assert_eq!(p.to_flake_follows_string(), "nixpkgs");
    }

    #[test]
    fn attr_path_to_flake_follows_string_two_segments() {
        let p = AttrPath::parse("crane.nixpkgs").unwrap();
        assert_eq!(p.to_flake_follows_string(), "crane/nixpkgs");
    }

    #[test]
    fn attr_path_to_flake_follows_string_dotted_segment_preserved() {
        let p = AttrPath::parse("\"hls-1.10\".nixpkgs").unwrap();
        // Dot inside a segment must NOT become a slash.
        assert_eq!(p.to_flake_follows_string(), "hls-1.10/nixpkgs");
    }
}
