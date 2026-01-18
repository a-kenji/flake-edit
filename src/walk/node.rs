use rnix::{Root, SyntaxKind, SyntaxNode};

use crate::change::Change;

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
pub fn should_remove_input(change: &Change, ctx: &Option<Context>, input_id: &str) -> bool {
    if !change.is_remove() {
        return false;
    }
    if let Some(id) = change.id()
        && id.to_string() == input_id
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
/// Handles nested input IDs like "poetry2nix.nixpkgs".
pub fn should_remove_nested_input(change: &Change, ctx: &Option<Context>, input_id: &str) -> bool {
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

/// Create a nested follows attribute node (inside an input block).
/// Example: `inputs.nixpkgs.follows = "nixpkgs";`
pub fn make_nested_follows_attr(input: &str, target: &str) -> Node {
    parse_node(&format!("inputs.{}.follows = \"{}\";", input, target))
}
