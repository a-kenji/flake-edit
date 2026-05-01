use rnix::{Root, SyntaxKind, SyntaxNode};

use crate::change::Change;
use crate::follows::{AttrPath, Segment};

use super::context::Context;

// Type alias for clearer function signatures
pub type Node = SyntaxNode;

/// Parse a string into a SyntaxNode.
pub fn parse_node(s: &str) -> Node {
    Root::parse(s).syntax()
}

/// Replace a child in a parent node and return the rebuilt SyntaxNode.
pub fn substitute_child(parent: &SyntaxNode, index: usize, new_child: &SyntaxNode) -> Node {
    let green = parent
        .green()
        .replace_child(index, new_child.green().into());
    parse_node(&green.to_string())
}

/// Create an empty syntax node, used when removing nodes.
pub fn empty_node() -> Node {
    Root::parse("").syntax()
}

/// Get a whitespace node copied from adjacent siblings, if present.
/// Checks previous sibling first, then next sibling.
pub fn get_sibling_whitespace(node: &SyntaxNode) -> Option<Node> {
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

/// Find the insertion index after a reference node, skipping past any trailing
/// inline whitespace and comments on the same line.
/// This prevents displacing trailing comments when inserting new nodes.
pub fn insertion_index_after(node: &SyntaxNode) -> usize {
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

/// Find the index of adjacent whitespace to strip after removing/replacing a child.
/// Returns the index of whitespace before the child if present, otherwise after.
pub fn adjacent_whitespace_index(child: &rnix::SyntaxElement) -> Option<usize> {
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

/// Check if an input should be removed based on the change and context.
pub fn should_remove_input(change: &Change, ctx: &Option<Context>, input_id: &Segment) -> bool {
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

/// Check if a nested input should be removed using context-aware matching.
/// Handles nested input IDs like `poetry2nix.nixpkgs`.
pub fn should_remove_nested_input(
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

/// Create a quoted string node.
/// Example: `"github:NixOS/nixpkgs"`
pub fn make_quoted_string(s: &str) -> Node {
    parse_node(&format!("\"{}\"", s))
}

/// Create a toplevel input URL attribute node.
/// Example: `inputs.nixpkgs.url = "github:NixOS/nixpkgs";`
pub fn make_toplevel_url_attr(id: &str, uri: &str) -> Node {
    parse_node(&format!("inputs.{}.url = \"{}\";", id, uri))
}

/// Create a toplevel input flake=false attribute node.
/// Example: `inputs.not_a_flake.flake = false;`
pub fn make_toplevel_flake_false_attr(id: &str) -> Node {
    parse_node(&format!("inputs.{}.flake = false;", id))
}

/// Create a nested input URL attribute node.
/// Example: `nixpkgs.url = "github:NixOS/nixpkgs";`
pub fn make_url_attr(id: &str, uri: &str) -> Node {
    parse_node(&format!("{}.url = \"{}\";", id, uri))
}

/// Create a nested input flake=false attribute node.
/// Example: `not_a_flake.flake = false;`
pub fn make_flake_false_attr(id: &str) -> Node {
    parse_node(&format!("{}.flake = false;", id))
}

/// Create a nested input URL attribute in attrset style.
/// Example: `vmsh = {\n    url = "github:mic92/vmsh";\n  };`
/// The `indent` parameter is the base indentation of the entry (e.g., "  " for 2-space indent).
pub fn make_attrset_url_attr(id: &str, uri: &str, indent: &str) -> Node {
    parse_node(&format!(
        "{} = {{\n{}  url = \"{}\";\n{}}};",
        id, indent, uri, indent
    ))
}

/// Create a nested input URL + flake=false attribute in attrset style.
pub fn make_attrset_url_flake_false_attr(id: &str, uri: &str, indent: &str) -> Node {
    parse_node(&format!(
        "{} = {{\n{}  url = \"{}\";\n{}  flake = false;\n{}}};",
        id, indent, uri, indent, indent
    ))
}

/// Shape of a `follows = ...` attribute being emitted into the CST.
///
/// Each variant captures both the attrpath layout and the surrounding
/// insertion context. Per-segment quoting is delegated to
/// [`Segment::render`] so dotted/leading-digit segments get their `"..."`
/// boundary automatically.
pub enum FollowsKind<'a> {
    /// `inputs.<id>.follows = "<target>";` - sibling of other
    /// `inputs.<...>.url = ...` attrs in the outer flake attr-set.
    TopLevelFlat { id: &'a Segment, target: &'a str },
    /// `inputs.<S0>.inputs.<S1>...inputs.<SN>.follows = "<target>";` (the
    /// fully-qualified flat shape used when the outer flake spells inputs
    /// out as `inputs.<parent>.url = ...`). The `path` carries every segment
    /// from the top-level input down to the leaf nested input (length
    /// `>= 2`).
    TopLevelNested { path: &'a AttrPath, target: &'a str },
    /// `<S0>.inputs.<S1>...inputs.<SN>.follows = "<target>";`, sibling
    /// inside an `inputs = { ... }` block where the parent input is declared
    /// as a flat `<parent>.url = ...` (no per-input `{ ... }` block).
    InputsBlockNested { path: &'a AttrPath, target: &'a str },
    /// `inputs.<R0>.inputs.<R1>...inputs.<RN>.follows = "<target>";`,
    /// sibling inside a parent input's `<parent> = { ... }` block, or inside
    /// an `inputs = { ... }` block at a sibling depth of the same shape.
    /// `rest` is the path below the enclosing parent (length `>= 1`).
    BlockNested {
        rest: &'a [Segment],
        target: &'a str,
    },
    /// `follows = "<target>";` - bare follows attr inside a parent
    /// input's `<parent> = { ... }` block.
    BlockBare { target: &'a str },
}

/// Render an attribute path of nested inputs as `S0.inputs.S1...inputs.SN`
/// (no leading `inputs.` and no trailing `.follows`). Per-segment quoting
/// is delegated to [`Segment::render`].
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
    pub fn emit(&self) -> Node {
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
        // Depth-3 source path emits two `.inputs.` separators.
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
