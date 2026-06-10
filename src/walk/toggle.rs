//! Detection and flipping of commented url alternates.
//!
//! An input is *toggleable* when a commented copy of its url binding sits
//! in the contiguous comment block directly above or below the active
//! binding (no blank line in between). Detection is parsing, not prefix
//! matching: stripped of its leading `#` (and at most one following
//! space), the comment must parse as a single binding that binds the same
//! url attribute at the position it occupies, with a string-literal
//! value. Anything else is prose and is ignored.
//!
//! Flips never move lines, only the comment marker: deactivating prefixes
//! the binding's source text with `# ` at its existing indentation, and
//! activating strips `#` plus at most one space. A trailing same-line
//! comment rides along verbatim in both directions, so toggling twice is
//! byte-identical (the one permitted normalization is `#x` becoming
//! `# x` after the first round trip).

use rnix::{SyntaxKind, SyntaxNode, SyntaxToken, TextRange, TextSize};

use crate::follows::strip_outer_quotes;
use crate::input::Input;

use super::node::{extract_indent, parse_node, trailing_inline_comments};

/// A commented alternate adjacent to an input's url binding.
#[derive(Debug, Clone)]
pub(crate) struct Alternate {
    /// Unquoted url string of the commented binding.
    pub(crate) url: String,
    /// The comment token holding the alternate.
    token: SyntaxToken,
}

/// Locate the `NODE_ATTRPATH_VALUE` binding `input`'s url, anchored on the
/// url value range the walker recorded. Returns `None` when the input has
/// no url binding (e.g. a follows-only input, whose recorded range points
/// at a `follows` binding instead).
pub(crate) fn url_binding(root: &SyntaxNode, input: &Input) -> Option<SyntaxNode> {
    if input.url().is_empty() || input.range.is_empty() {
        return None;
    }
    let range = TextRange::new(
        TextSize::from(input.range.start as u32),
        TextSize::from(input.range.end as u32),
    );
    if !root.text_range().contains_range(range) {
        return None;
    }
    let covering = root.covering_element(range);
    let node = match covering {
        rnix::NodeOrToken::Node(node) => node,
        rnix::NodeOrToken::Token(token) => token.parent()?,
    };
    let binding = node
        .ancestors()
        .find(|a| a.kind() == SyntaxKind::NODE_ATTRPATH_VALUE)?;
    let segments = attrpath_segments(&binding)?;
    (segments.last().map(String::as_str) == Some("url")).then_some(binding)
}

/// Unquoted attrpath segments of a binding, e.g. `["rust-overlay", "url"]`.
fn attrpath_segments(binding: &SyntaxNode) -> Option<Vec<String>> {
    let attrpath = binding
        .children()
        .find(|c| c.kind() == SyntaxKind::NODE_ATTRPATH)?;
    Some(
        attrpath
            .children()
            .map(|c| strip_outer_quotes(&c.to_string()).to_string())
            .collect(),
    )
}

/// Collect the alternates stored next to `binding`, in file order: the
/// contiguous own-line comment block above, then the one below.
pub(crate) fn alternates(binding: &SyntaxNode) -> Vec<Alternate> {
    let Some(expected) = attrpath_segments(binding) else {
        return Vec::new();
    };

    let mut above = Vec::new();
    let mut cursor = binding.prev_sibling_or_token();
    while let Some(el) = cursor.clone() {
        match el.kind() {
            SyntaxKind::TOKEN_WHITESPACE => {
                if el.to_string().matches('\n').count() >= 2 {
                    break;
                }
            }
            SyntaxKind::TOKEN_COMMENT => {
                // A comment with line content before it is a trailing
                // comment of an earlier statement. The block ends there.
                if !on_own_line(&el) {
                    break;
                }
                if let Some(token) = el.as_token()
                    && let Some(url) = parse_alternate(&token.to_string(), &expected)
                {
                    above.push(Alternate {
                        url,
                        token: token.clone(),
                    });
                }
            }
            _ => break,
        }
        cursor = el.prev_sibling_or_token();
    }
    above.reverse();

    let mut found = above;
    let mut cursor = binding.next_sibling_or_token();
    while let Some(el) = cursor.clone() {
        match el.kind() {
            SyntaxKind::TOKEN_WHITESPACE => {
                if el.to_string().matches('\n').count() >= 2 {
                    break;
                }
            }
            SyntaxKind::TOKEN_COMMENT => {
                // Skip rather than break: a non-own-line comment here is
                // the binding's own trailing comment, and the block may
                // still start on the next line.
                if on_own_line(&el)
                    && let Some(token) = el.as_token()
                    && let Some(url) = parse_alternate(&token.to_string(), &expected)
                {
                    found.push(Alternate {
                        url,
                        token: token.clone(),
                    });
                }
            }
            _ => break,
        }
        cursor = el.next_sibling_or_token();
    }
    found
}

/// Whether `el` starts its line: its previous sibling element is
/// line-breaking whitespace (or it is the first child).
fn on_own_line(el: &rnix::SyntaxElement) -> bool {
    match el.prev_sibling_or_token() {
        None => true,
        Some(prev) => {
            prev.kind() == SyntaxKind::TOKEN_WHITESPACE && prev.to_string().contains('\n')
        }
    }
}

/// Strip the comment marker: a leading `#` and at most one following space.
fn uncomment(comment: &str) -> &str {
    let body = comment.strip_prefix('#').unwrap_or(comment);
    body.strip_prefix(' ').unwrap_or(body)
}

/// Parse `comment` as an alternate of the binding whose unquoted attrpath
/// is `expected`. Returns the unquoted url on success.
///
/// The body is parsed inside a synthetic `{ ... }` because a bare binding
/// is not a valid root expression. The wrapper makes "parses as a single
/// binding" checkable with an error-free parse.
fn parse_alternate(comment: &str, expected: &[String]) -> Option<String> {
    if !comment.starts_with('#') {
        return None;
    }
    let body = uncomment(comment);
    let parse = rnix::Root::parse(&format!("{{\n{body}\n}}"));
    if !parse.errors().is_empty() {
        return None;
    }
    let attr_set = parse.syntax().first_child()?;
    if attr_set.kind() != SyntaxKind::NODE_ATTR_SET {
        return None;
    }
    let mut bindings = attr_set.children();
    let binding = bindings.next()?;
    if bindings.next().is_some() || binding.kind() != SyntaxKind::NODE_ATTRPATH_VALUE {
        return None;
    }
    let segments = attrpath_segments(&binding)?;
    if segments != expected {
        return None;
    }
    let attrpath = binding
        .children()
        .find(|c| c.kind() == SyntaxKind::NODE_ATTRPATH)?;
    let value = attrpath.next_sibling()?;
    if value.kind() != SyntaxKind::NODE_STRING {
        return None;
    }
    Some(strip_outer_quotes(&value.to_string()).to_string())
}

/// The commented form of `binding`'s line: the `# `-prefixed source text
/// (with any trailing same-line comment riding along verbatim) and the
/// indices of the tail tokens that merge into it.
fn deactivated_line(binding: &SyntaxNode) -> (String, Vec<usize>) {
    let element: rnix::SyntaxElement = binding.clone().into();
    let tail = trailing_inline_comments(&element);
    let mut text = format!("# {binding}");
    for token in &tail {
        text.push_str(&token.to_string());
    }
    (text, tail.iter().map(|token| token.index()).collect())
}

/// Activate the stored `alternate` and deactivate the active `binding`.
/// Both lines keep their position. Only the comment marker moves.
pub(crate) fn flip(parent: &SyntaxNode, binding: &SyntaxNode, alternate: &Alternate) -> SyntaxNode {
    let activated = parse_node(uncomment(&alternate.token.to_string()));
    let (comment_text, tail) = deactivated_line(binding);
    // Replacements are one-to-one and leave sibling indices valid. The
    // tail removals shift only children after the binding, so they run
    // last and in descending order.
    let mut green = parent
        .green()
        .replace_child(alternate.token.index(), activated.green().into());
    green = green.replace_child(binding.index(), parse_node(&comment_text).green().into());
    for index in tail.iter().rev() {
        green = green.remove_child(*index);
    }
    SyntaxNode::new_root(parent.replace_with(green))
}

/// Delete the stored `alternate`'s line: the comment token and the
/// whitespace before it (the newline and indentation), so the
/// surrounding lines keep their positions.
pub(crate) fn remove_alternate(parent: &SyntaxNode, alternate: &Alternate) -> SyntaxNode {
    let token = &alternate.token;
    let mut green = parent.green().remove_child(token.index());
    if let Some(prev) = token.prev_sibling_or_token()
        && prev.kind() == SyntaxKind::TOKEN_WHITESPACE
    {
        green = green.remove_child(prev.index());
    }
    SyntaxNode::new_root(parent.replace_with(green))
}

/// Activate the stored `alternate` and delete the active `binding`'s line
/// entirely: the binding, its trailing same-line comments, and the
/// whitespace before it. The removing counterpart of [`flip`], dropping
/// the deactivated variant instead of keeping it as a comment.
pub(crate) fn flip_remove(
    parent: &SyntaxNode,
    binding: &SyntaxNode,
    alternate: &Alternate,
) -> SyntaxNode {
    let activated = parse_node(uncomment(&alternate.token.to_string()));
    let element: rnix::SyntaxElement = binding.clone().into();
    let mut to_remove: Vec<usize> = trailing_inline_comments(&element)
        .iter()
        .map(|t| t.index())
        .collect();
    to_remove.push(binding.index());
    if let Some(prev) = binding.prev_sibling_or_token()
        && prev.kind() == SyntaxKind::TOKEN_WHITESPACE
    {
        to_remove.push(prev.index());
    }
    to_remove.sort_unstable();
    // The replacement is one-to-one, so the collected indices stay valid.
    // The removals then run highest-first to keep the lower ones valid.
    let mut green = parent
        .green()
        .replace_child(alternate.token.index(), activated.green().into());
    for index in to_remove.iter().rev() {
        green = green.remove_child(*index);
    }
    SyntaxNode::new_root(parent.replace_with(green))
}

/// Store `uri` as the new active url: the active `binding` is commented in
/// place and the new url binding is written directly below it, at the same
/// indentation and with the same attrpath spelling.
pub(crate) fn synthesize(parent: &SyntaxNode, binding: &SyntaxNode, uri: &str) -> SyntaxNode {
    let attrpath_text = binding
        .children()
        .find(|c| c.kind() == SyntaxKind::NODE_ATTRPATH)
        .map(|c| c.to_string())
        .unwrap_or_default();
    let indent = binding
        .prev_sibling_or_token()
        .filter(|t| t.kind() == SyntaxKind::TOKEN_WHITESPACE)
        .map(|t| extract_indent(&t.to_string()).to_string())
        .unwrap_or_default();

    let (comment_text, tail) = deactivated_line(binding);
    // Order matters: the replacement is one-to-one and the tail removals
    // only touch children after the binding, so `binding.index()` stays
    // valid throughout and the inserts below land directly after the
    // commented line. Reordering these steps breaks the index arithmetic.
    let mut green = parent
        .green()
        .replace_child(binding.index(), parse_node(&comment_text).green().into());
    for index in tail.iter().rev() {
        green = green.remove_child(*index);
    }
    let new_line = parse_node(&format!("{attrpath_text} = \"{uri}\";"));
    green = green.insert_child(
        binding.index() + 1,
        parse_node(&format!("\n{indent}")).green().into(),
    );
    green = green.insert_child(binding.index() + 2, new_line.green().into());
    SyntaxNode::new_root(parent.replace_with(green))
}
