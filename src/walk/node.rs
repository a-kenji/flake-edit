use rnix::{Root, SyntaxKind, SyntaxNode};

use crate::change::Change;
use crate::follows::{AttrPath, Segment};

use super::context::Context;

pub(crate) type Node = SyntaxNode;

/// Parse `s` as a Nix expression and return its [`SyntaxNode`].
pub(crate) fn parse_node(s: &str) -> Node {
    Root::parse(s).syntax()
}

/// Replace `parent`'s child at `index` with `new_child` and return the rebuilt node.
pub(crate) fn substitute_child(parent: &SyntaxNode, index: usize, new_child: &SyntaxNode) -> Node {
    let green = parent
        .green()
        .replace_child(index, new_child.green().into());
    parse_node(&green.to_string())
}

/// Empty syntax node used as a removal placeholder.
pub(crate) fn empty_node() -> Node {
    Root::parse("").syntax()
}

/// Whether `attr_set` carries no semantic content (no bindings, no comments).
///
/// Returns true only for `NODE_ATTR_SET` nodes whose body contains nothing
/// but braces and whitespace. Comments count as user-authored content and
/// suppress the empty verdict, so the prune pass must not collapse a block
/// the user populated with intent. A `NODE_ROOT` wrapper produced by
/// [`parse_node`] is unwrapped to its inner attrset so re-parsed fragments
/// flow through the same predicate.
pub(crate) fn is_attrset_content_empty(node: &SyntaxNode) -> bool {
    let attr_set = if node.kind() == SyntaxKind::NODE_ROOT {
        match node.first_child() {
            Some(inner) => inner,
            None => return false,
        }
    } else {
        node.clone()
    };
    if attr_set.kind() != SyntaxKind::NODE_ATTR_SET {
        return false;
    }
    if attr_set
        .children()
        .any(|c| c.kind() == SyntaxKind::NODE_ATTRPATH_VALUE)
    {
        return false;
    }
    !attr_set
        .children_with_tokens()
        .any(|t| t.kind() == SyntaxKind::TOKEN_COMMENT)
}

/// Whitespace node copied from `node`'s previous sibling (or next, as fallback).
pub(crate) fn get_sibling_whitespace(node: &SyntaxNode) -> Option<Node> {
    if let Some(prev) = node.prev_sibling_or_token()
        && prev.kind() == SyntaxKind::TOKEN_WHITESPACE
    {
        return Some(parse_node(prev.as_token().unwrap().green().text()));
    }
    if let Some(next) = node.next_sibling_or_token()
        && next.kind() == SyntaxKind::TOKEN_WHITESPACE
    {
        return Some(parse_node(next.as_token().unwrap().green().text()));
    }
    None
}

/// Insertion index after `node`, skipping trailing same-line whitespace and comments.
///
/// Stops at the first newline so trailing comments on the reference line stay attached
/// to it instead of getting displaced by the inserted node.
pub(crate) fn insertion_index_after(node: &SyntaxNode) -> usize {
    let element: rnix::SyntaxElement = node.clone().into();
    let mut cursor = element.next_sibling_or_token();
    let mut last_index = node.index() + 1;
    while let Some(ref tok) = cursor {
        match tok.kind() {
            SyntaxKind::TOKEN_WHITESPACE => {
                let text = tok.to_string();
                if text.contains('\n') {
                    break;
                }
                last_index = tok.index() + 1;
            }
            SyntaxKind::TOKEN_COMMENT => {
                last_index = tok.index() + 1;
            }
            _ => break,
        }
        cursor = tok.next_sibling_or_token();
    }
    last_index
}

/// Index of `child`'s adjacent whitespace token (preferring the previous sibling)
/// for stripping after a removal or replacement.
pub(crate) fn adjacent_whitespace_index(child: &rnix::SyntaxElement) -> Option<usize> {
    if let Some(prev) = child.prev_sibling_or_token()
        && prev.kind() == SyntaxKind::TOKEN_WHITESPACE
    {
        Some(prev.index())
    } else if let Some(next) = child.next_sibling_or_token()
        && next.kind() == SyntaxKind::TOKEN_WHITESPACE
    {
        Some(next.index())
    } else {
        None
    }
}

/// Whether `input_id` should be removed under `change` and `ctx`.
pub(crate) fn should_remove_input(
    change: &Change,
    ctx: &Option<Context>,
    input_id: &Segment,
) -> bool {
    if !change.is_remove() {
        return false;
    }
    if let Some(id) = change.id()
        && id.input() == input_id
        && id.follows().is_none()
    {
        return true;
    }
    if let Some(ctx) = ctx
        && ctx.first_matches(input_id)
    {
        return true;
    }
    false
}

/// Whether a nested input should be removed, using `ctx` for dotted IDs like
/// `poetry2nix.nixpkgs`.
pub(crate) fn should_remove_nested_input(
    change: &Change,
    ctx: &Option<Context>,
    input_id: &Segment,
) -> bool {
    if !change.is_remove() {
        return false;
    }
    if let Some(id) = change.id() {
        return id.matches_with_ctx(input_id, ctx.clone());
    }
    false
}

/// Quoted string node, e.g. `"github:NixOS/nixpkgs"`.
pub(crate) fn make_quoted_string(s: &str) -> Node {
    parse_node(&format!("\"{}\"", s))
}

/// Top-level URL attribute, e.g. `inputs.nixpkgs.url = "github:NixOS/nixpkgs";`.
pub(crate) fn make_toplevel_url_attr(id: &str, uri: &str) -> Node {
    parse_node(&format!("inputs.{}.url = \"{}\";", id, uri))
}

/// Top-level `flake = false` attribute, e.g. `inputs.not_a_flake.flake = false;`.
pub(crate) fn make_toplevel_flake_false_attr(id: &str) -> Node {
    parse_node(&format!("inputs.{}.flake = false;", id))
}

/// Nested URL attribute, e.g. `nixpkgs.url = "github:NixOS/nixpkgs";`.
pub(crate) fn make_url_attr(id: &str, uri: &str) -> Node {
    parse_node(&format!("{}.url = \"{}\";", id, uri))
}

/// Nested `flake = false` attribute, e.g. `not_a_flake.flake = false;`.
pub(crate) fn make_flake_false_attr(id: &str) -> Node {
    parse_node(&format!("{}.flake = false;", id))
}

/// Attrset-style URL attribute, e.g. `vmsh = { url = "github:mic92/vmsh"; };`.
///
/// `indent` is the base indentation of the entry (e.g., `"  "` for 2-space indent).
/// The inner attribute gets one extra level.
pub(crate) fn make_attrset_url_attr(id: &str, uri: &str, indent: &str) -> Node {
    parse_node(&format!(
        "{} = {{\n{}  url = \"{}\";\n{}}};",
        id, indent, uri, indent
    ))
}

/// Attrset-style URL plus `flake = false`.
pub(crate) fn make_attrset_url_flake_false_attr(id: &str, uri: &str, indent: &str) -> Node {
    parse_node(&format!(
        "{} = {{\n{}  url = \"{}\";\n{}  flake = false;\n{}}};",
        id, indent, uri, indent, indent
    ))
}

/// Shape of a `follows = ...` attribute to splice into the CST.
///
/// Each variant captures both the attrpath layout and the surrounding insertion context.
/// Per-segment quoting goes through [`Segment::render`] so dotted or leading-digit
/// segments pick up `"..."` automatically.
pub(crate) enum FollowsKind<'a> {
    /// `inputs.<id>.follows = "<target>";`, sibling of other `inputs.<...>.url = ...`
    /// attrs in the outer flake attr-set.
    TopLevelFlat { id: &'a Segment, target: &'a str },
    /// `inputs.<S0>.inputs.<S1>...inputs.<SN>.follows = "<target>";`, the fully-qualified
    /// flat shape used when the outer flake spells inputs as `inputs.<parent>.url = ...`.
    /// `path` covers every segment from the top-level input to the leaf nested input
    /// (length `>= 2`).
    TopLevelNested { path: &'a AttrPath, target: &'a str },
    /// `<S0>.inputs.<S1>...inputs.<SN>.follows = "<target>";`, sibling inside an
    /// `inputs = { ... }` block where the parent input is declared as a flat
    /// `<parent>.url = ...` (no per-input `{ ... }` block).
    InputsBlockNested { path: &'a AttrPath, target: &'a str },
    /// `inputs.<R0>.inputs.<R1>...inputs.<RN>.follows = "<target>";`, sibling inside
    /// a parent input's `<parent> = { ... }` block, or inside an `inputs = { ... }`
    /// block at a sibling depth of the same shape. `rest` is the path below the
    /// enclosing parent (length `>= 1`).
    BlockNested {
        rest: &'a [Segment],
        target: &'a str,
    },
    /// `follows = "<target>";`, bare follows attr inside a parent input's
    /// `<parent> = { ... }` block.
    BlockBare { target: &'a str },
}

/// Render `segments` as `S0.inputs.S1...inputs.SN` (no leading `inputs.`, no trailing
/// `.follows`). Per-segment quoting goes through [`Segment::render`].
fn render_inputs_chain(segments: &[Segment]) -> String {
    let mut out = String::new();
    for (i, seg) in segments.iter().enumerate() {
        if i > 0 {
            out.push_str(".inputs.");
        }
        out.push_str(&seg.render());
    }
    out
}

impl FollowsKind<'_> {
    /// Render the variant to a [`SyntaxNode`] ready to splice into the CST.
    pub(crate) fn emit(&self) -> Node {
        match self {
            FollowsKind::TopLevelFlat { id, target } => {
                parse_node(&format!("inputs.{}.follows = \"{}\";", id.render(), target))
            }
            FollowsKind::TopLevelNested { path, target } => {
                let chain = render_inputs_chain(path.segments());
                parse_node(&format!("inputs.{chain}.follows = \"{target}\";"))
            }
            FollowsKind::InputsBlockNested { path, target } => {
                let chain = render_inputs_chain(path.segments());
                parse_node(&format!("{chain}.follows = \"{target}\";"))
            }
            FollowsKind::BlockNested { rest, target } => {
                let chain = render_inputs_chain(rest);
                parse_node(&format!("inputs.{chain}.follows = \"{target}\";"))
            }
            FollowsKind::BlockBare { target } => parse_node(&format!("follows = \"{}\";", target)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(s: &str) -> Segment {
        Segment::from_unquoted(s).expect("valid segment")
    }

    #[test]
    fn follows_kind_top_level_flat_bare_ident() {
        let id = seg("nixpkgs");
        let node = FollowsKind::TopLevelFlat {
            id: &id,
            target: "github:NixOS/nixpkgs",
        }
        .emit();
        assert_eq!(
            node.to_string(),
            "inputs.nixpkgs.follows = \"github:NixOS/nixpkgs\";"
        );
    }

    #[test]
    fn follows_kind_top_level_flat_quotes_dotted_segment() {
        let id = seg("hls-1.10");
        let node = FollowsKind::TopLevelFlat {
            id: &id,
            target: "nixpkgs",
        }
        .emit();
        assert_eq!(
            node.to_string(),
            "inputs.\"hls-1.10\".follows = \"nixpkgs\";"
        );
    }

    fn path(s: &str) -> AttrPath {
        AttrPath::parse(s).expect("valid attrpath")
    }

    #[test]
    fn follows_kind_top_level_nested() {
        let p = path("crane.nixpkgs");
        let node = FollowsKind::TopLevelNested {
            path: &p,
            target: "nixpkgs",
        }
        .emit();
        assert_eq!(
            node.to_string(),
            "inputs.crane.inputs.nixpkgs.follows = \"nixpkgs\";"
        );
    }

    #[test]
    fn follows_kind_top_level_nested_quotes_dotted_parent() {
        let p = path("\"hls-1.10\".nixpkgs");
        let node = FollowsKind::TopLevelNested {
            path: &p,
            target: "nixpkgs",
        }
        .emit();
        assert_eq!(
            node.to_string(),
            "inputs.\"hls-1.10\".inputs.nixpkgs.follows = \"nixpkgs\";"
        );
    }

    #[test]
    fn follows_kind_top_level_nested_depth_three() {
        let p = path("neovim.nixvim.flake-parts");
        let node = FollowsKind::TopLevelNested {
            path: &p,
            target: "flake-parts",
        }
        .emit();
        assert_eq!(
            node.to_string(),
            "inputs.neovim.inputs.nixvim.inputs.flake-parts.follows = \"flake-parts\";"
        );
    }

    #[test]
    fn follows_kind_inputs_block_nested() {
        let p = path("harmonia.nixpkgs");
        let node = FollowsKind::InputsBlockNested {
            path: &p,
            target: "nixpkgs",
        }
        .emit();
        assert_eq!(
            node.to_string(),
            "harmonia.inputs.nixpkgs.follows = \"nixpkgs\";"
        );
    }

    #[test]
    fn follows_kind_inputs_block_nested_depth_three() {
        let p = path("neovim.nixvim.flake-parts");
        let node = FollowsKind::InputsBlockNested {
            path: &p,
            target: "flake-parts",
        }
        .emit();
        assert_eq!(
            node.to_string(),
            "neovim.inputs.nixvim.inputs.flake-parts.follows = \"flake-parts\";"
        );
    }

    #[test]
    fn follows_kind_block_nested() {
        let rest = [seg("nixpkgs")];
        let node = FollowsKind::BlockNested {
            rest: &rest,
            target: "nixpkgs",
        }
        .emit();
        assert_eq!(node.to_string(), "inputs.nixpkgs.follows = \"nixpkgs\";");
    }

    #[test]
    fn follows_kind_block_nested_quotes_dotted() {
        let rest = [seg("hls-1.10")];
        let node = FollowsKind::BlockNested {
            rest: &rest,
            target: "nixpkgs",
        }
        .emit();
        assert_eq!(
            node.to_string(),
            "inputs.\"hls-1.10\".follows = \"nixpkgs\";"
        );
    }

    #[test]
    fn follows_kind_block_nested_depth_two() {
        // Inside parent's `{ ... }` block, the rest is everything below it.
        let rest = [seg("nixvim"), seg("flake-parts")];
        let node = FollowsKind::BlockNested {
            rest: &rest,
            target: "flake-parts",
        }
        .emit();
        assert_eq!(
            node.to_string(),
            "inputs.nixvim.inputs.flake-parts.follows = \"flake-parts\";"
        );
    }

    #[test]
    fn follows_kind_block_bare() {
        let node = FollowsKind::BlockBare { target: "nixpkgs" }.emit();
        assert_eq!(node.to_string(), "follows = \"nixpkgs\";");
    }
}
