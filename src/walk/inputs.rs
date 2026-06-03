use std::collections::HashMap;

use rnix::{SyntaxKind, SyntaxNode};

use crate::change::Change;
use crate::follows::path::{follows_idents_bare, follows_idents_prefixed};
use crate::follows::{AttrPath, Segment, strip_outer_quotes};
use crate::input::Input;

use super::context::Context;
use super::node::{
    FollowsKind, adjacent_whitespace_index, empty_node, extract_indent, get_sibling_whitespace,
    insertion_index_after, is_attrset_content_empty, last_line_with_newline, make_attrset_url_attr,
    make_attrset_url_flake_false_attr, make_flake_false_attr, make_quoted_string, make_url_attr,
    parse_node, remove_child_with_whitespace, should_remove_input, should_remove_nested_input,
    substitute_child, trailing_inline_comment_indices, uses_attrset_style,
};

/// Insert or update `inputs[id]` from a parsed `Input`.
///
/// When `ctx` carries an enclosing input, the `input` is interpreted as a follows
/// edge attached to that owner instead of a top-level entry. Only depth-1 follows
/// shapes (`<owner>.<id>.follows = ...`) hit this path. Deeper shapes are parsed up
/// front in [`handle_attrpath_follows`] and bypass this helper.
pub(crate) fn insert_with_ctx(
    inputs: &mut HashMap<String, Input>,
    id: Segment,
    input: Input,
    ctx: &Option<Context>,
) {
    if let Some(ctx) = ctx {
        if let Some(follows) = ctx.first() {
            // The follows target arrives as the `input.url` token.
            let target = AttrPath::parse_follows_target(&input.url, &id);
            let key = follows.as_str().to_string();
            let nested_path = AttrPath::new(id.clone());
            if let Some(node) = inputs.get_mut(&key) {
                node.push_indirect_follows(nested_path, target);
            } else {
                let mut stub = Input::new(follows.clone());
                stub.follows.push(crate::input::Follows::Indirect {
                    path: nested_path,
                    target,
                });
                inputs.insert(key, stub);
            }
        }
    } else {
        let key = id.as_str().to_string();
        if let Some(node) = inputs.get_mut(&key) {
            if !input.url.is_empty() {
                node.url = input.url;
                node.range = input.range;
            }
            if !input.flake {
                node.flake = input.flake;
            }
        } else {
            inputs.insert(key, input);
        }
    }
}

/// Walk the `inputs` section of a `flake.nix`, applying `change` and recording
/// every traversed input into `inputs`.
pub(crate) fn walk_inputs(
    inputs: &mut HashMap<String, Input>,
    node: SyntaxNode,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    if node.kind() == SyntaxKind::NODE_ATTRPATH
        && let Some(result) = handle_attrpath_follows(inputs, &node, change)
    {
        return Some(result);
    }

    match change {
        Change::None => apply_none(inputs, &node, ctx, change),
        Change::Add { .. } => apply_add(inputs, node, ctx, change),
        Change::Remove { .. } => apply_remove(inputs, &node, ctx, change),
        Change::Change { .. } => apply_change_uri(inputs, &node, ctx, change),
        Change::Follows { .. } => apply_follows(inputs, node, ctx, change),
    }
}

/// `FlakeEdit::list` drives a `Change::None` walk purely for the side effect
/// of populating the `inputs` map via the per-attr handlers, so this branch
/// must traverse children even though it never rewrites.
fn apply_none(
    inputs: &mut HashMap<String, Input>,
    node: &SyntaxNode,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    walk_children(inputs, node, ctx, change)
}

fn walk_children(
    inputs: &mut HashMap<String, Input>,
    node: &SyntaxNode,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    for child in node.children_with_tokens() {
        match child.kind() {
            SyntaxKind::NODE_ATTRPATH_VALUE => {
                if let Some(result) = handle_child_attrpath_value(inputs, node, &child, ctx, change)
                {
                    return Some(result);
                }
            }
            SyntaxKind::NODE_IDENT => {
                if let Some(result) = handle_child_ident(inputs, &child, ctx, change) {
                    return Some(result);
                }
            }
            _ => {}
        }
    }
    None
}

fn apply_remove(
    inputs: &mut HashMap<String, Input>,
    node: &SyntaxNode,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    walk_children(inputs, node, ctx, change)
}

fn apply_change_uri(
    inputs: &mut HashMap<String, Input>,
    node: &SyntaxNode,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    walk_children(inputs, node, ctx, change)
}

/// An empty `inputs = { }` block has no `NODE_ATTRPATH_VALUE` children for
/// [`walk_children`] to splice into, so the rebuild step below kicks in
/// only when traversal returned nothing.
fn apply_add(
    inputs: &mut HashMap<String, Input>,
    node: SyntaxNode,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    if let Some(result) = walk_children(inputs, &node, ctx, change) {
        return Some(result);
    }

    let Change::Add {
        id: Some(id),
        uri: Some(uri),
        flake,
    } = change
    else {
        return None;
    };
    let id = id.input().as_str();

    if node.kind() != SyntaxKind::NODE_ATTR_SET || ctx.is_some() {
        return None;
    }
    if node
        .children()
        .any(|c| c.kind() == SyntaxKind::NODE_ATTRPATH_VALUE)
    {
        return None;
    }

    Some(insert_into_empty_inputs(&node, id, uri, *flake))
}

/// Indentation copies the whitespace preceding the `inputs` attrpath-value
/// node so the inserted entry lines up with whatever the user already wrote
/// elsewhere in the file. Contents indent one level deeper.
fn insert_into_empty_inputs(node: &SyntaxNode, id: &str, uri: &str, flake: bool) -> SyntaxNode {
    let base_indent = node
        .parent()
        .and_then(|p| p.prev_sibling_or_token())
        .filter(|t| t.kind() == SyntaxKind::TOKEN_WHITESPACE)
        .map(|t| extract_indent(&t.to_string()).to_string())
        .unwrap_or_else(|| "  ".to_string());
    let entry_indent = format!("\n{}  ", base_indent);
    let closing_indent = format!("\n{}", base_indent);

    let uri_node = make_url_attr(id, uri);

    // Drop any whitespace already sitting between the braces, then rebuild
    // the contents from scratch.
    let ws_index = node
        .children_with_tokens()
        .find(|t| t.kind() == SyntaxKind::TOKEN_WHITESPACE)
        .map(|t| t.index());

    let mut green = if let Some(idx) = ws_index {
        node.green().remove_child(idx)
    } else {
        node.green().into_owned()
    };

    let brace_index = green
        .children()
        .position(|c| c.as_token().map(|t| t.text() == "}").unwrap_or(false))
        .unwrap_or(green.children().count());

    green = green.insert_child(brace_index, uri_node.green().into());
    green = green.insert_child(brace_index, parse_node(&entry_indent).green().into());

    let mut offset = 2;
    if !flake {
        let no_flake = make_flake_false_attr(id);
        green = green.insert_child(
            brace_index + offset,
            parse_node(&entry_indent).green().into(),
        );
        offset += 1;
        green = green.insert_child(brace_index + offset, no_flake.green().into());
        offset += 1;
    }

    green = green.insert_child(
        brace_index + offset,
        parse_node(&closing_indent).green().into(),
    );

    SyntaxNode::new_root(green)
}

/// When the parent is declared in attrset shape (`parent = { ... }`)
/// [`handle_input_attr_set`] owns the follows insertion via the child
/// traversal step. This function only handles the flat-toplevel shape
/// (`parent.url = "..."`), where the follows entry must be spliced next
/// to the `url` declaration before traversal happens.
fn apply_follows(
    inputs: &mut HashMap<String, Input>,
    node: SyntaxNode,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    let Change::Follows { input, target } = change else {
        return None;
    };

    if ctx.is_none()
        && node.kind() == SyntaxKind::NODE_ATTR_SET
        && let Some(result) = insert_flat_toplevel_follows(inputs, &node, input, target)
    {
        return result;
    }

    walk_children(inputs, &node, ctx, change)
}

/// The `Option<Option<_>>` return distinguishes "this branch claims the
/// rewrite" from "this branch does not apply". `Some(Some(node))` means
/// the rewrite has been performed (or the desired follows already exists,
/// in which case `node` is the unchanged input). `Some(None)` is reserved
/// for hits that intentionally short-circuit without producing a rewrite.
/// `None` lets the caller fall through to child traversal.
fn insert_flat_toplevel_follows(
    inputs: &HashMap<String, Input>,
    node: &SyntaxNode,
    input: &crate::change::ChangeId,
    target: &AttrPath,
) -> Option<Option<SyntaxNode>> {
    let full_path = input.path();
    let parent_id = input.input();
    let parent_id_str = parent_id.as_str();
    let target_str = target.to_flake_follows_string();

    if full_path.len() >= 2 {
        let parent_exists = inputs.contains_key(parent_id_str);
        if parent_exists && !flat_input_has_nested_block(node, parent_id_str) {
            if let Some(result) =
                find_existing_flat_follows(node, full_path.segments(), &target_str)
            {
                return Some(result);
            }
            let follows_node = FollowsKind::InputsBlockNested {
                path: full_path,
                target: &target_str,
            }
            .emit();
            // `None` when the parent isn't declared in this block.
            // `Walker::handle_follows_flat_toplevel` owns the split-declaration
            // placement instead.
            let inserted = insert_after_flat_input(node, parent_id_str, &follows_node);
            if inserted.is_some() {
                return Some(inserted);
            }
        }
    }

    if full_path.len() == 1 && !flat_input_has_nested_block(node, parent_id_str) {
        let follows_node = FollowsKind::TopLevelFlat {
            id: parent_id,
            target: &target_str,
        }
        .emit();
        let inserted = insert_after_flat_input(node, parent_id_str, &follows_node);
        if inserted.is_some() {
            return Some(inserted);
        }
    }

    None
}

/// Discriminator between the two ways a parent input can be declared.
/// Returns `true` for `parent = { ... }` (attrset) and `false` for
/// `parent.url = "..."` (flat). The flat-toplevel follows insertion path
/// must skip the attrset case so [`handle_input_attr_set`] can splice the
/// follows entry into the parent's own block instead.
fn flat_input_has_nested_block(node: &SyntaxNode, parent_id_str: &str) -> bool {
    node.children().any(|child| {
        if child.kind() != SyntaxKind::NODE_ATTRPATH_VALUE {
            return false;
        }
        child
            .first_child()
            .and_then(|attrpath| attrpath.first_child())
            .map(|first_ident| strip_outer_quotes(&first_ident.to_string()) == parent_id_str)
            .unwrap_or(false)
            && child
                .children()
                .any(|c| c.kind() == SyntaxKind::NODE_ATTR_SET)
    })
}

/// Reuses the whitespace before the anchor attr so the inserted line
/// inherits the user's indentation. Returns `None` when `parent_id_str`
/// has no flat-style declaration in this block.
fn insert_after_flat_input(
    node: &SyntaxNode,
    parent_id_str: &str,
    follows_node: &SyntaxNode,
) -> Option<SyntaxNode> {
    let children: Vec<_> = node.children().collect();
    let ref_child = children.iter().rev().find(|child| {
        child
            .first_child()
            .and_then(|attrpath| attrpath.first_child())
            .map(|first_ident| strip_outer_quotes(&first_ident.to_string()) == parent_id_str)
            .unwrap_or(false)
    })?;

    let insert_index = insertion_index_after(ref_child);
    let mut green = node
        .green()
        .insert_child(insert_index, follows_node.green().into());

    if let Some(whitespace) = get_sibling_whitespace(ref_child) {
        let ws_str = whitespace.to_string();
        let ws_node = parse_node(last_line_with_newline(&ws_str));
        green = green.insert_child(insert_index, ws_node.green().into());
    }

    Some(SyntaxNode::new_root(green))
}

/// Handle a flat-style URL attribute (`inputs.foo.url = "..."`), returning the
/// replacement node when `change` modifies it.
fn handle_flat_url(
    inputs: &mut HashMap<String, Input>,
    input_id: &SyntaxNode,
    url: &SyntaxNode,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    let id_seg = Segment::from_syntax_or_sentinel(input_id);
    let id_str = id_seg.as_str().to_string();
    let input = Input::with_url(id_seg.clone(), url.to_string(), url.text_range());
    insert_with_ctx(inputs, id_seg.clone(), input, ctx);

    if should_remove_input(change, ctx, &id_seg) {
        return Some(empty_node());
    }

    if let Change::Change {
        id: Some(change_id),
        uri: Some(new_uri),
        ..
    } = change
        && change_id.input().as_str() == id_str
        && change_id.follows().is_none()
    {
        return Some(make_quoted_string(new_uri));
    }

    None
}

/// Handle a flat-style flake attribute (`inputs.foo.flake = false`), returning
/// the replacement node when `change` removes the input.
fn handle_flat_flake(
    input_id: &SyntaxNode,
    _is_flake: &SyntaxNode,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    let id_seg = Segment::from_syntax_or_sentinel(input_id);

    if should_remove_input(change, ctx, &id_seg) {
        return Some(empty_node());
    }

    None
}

/// Handle a nested input declaration (`inputs.foo = { url = "..."; ... }`),
/// returning the replacement node when `change` modifies it.
fn handle_nested_input(
    inputs: &mut HashMap<String, Input>,
    input_id: &SyntaxNode,
    nested_attr: &SyntaxNode,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    let id_seg = Segment::from_syntax_or_sentinel(input_id);

    for attr in nested_attr.children() {
        for binding in attr.children() {
            if binding.to_string() == "url" {
                let url = binding.next_sibling().unwrap();
                let input = Input::with_url(id_seg.clone(), url.to_string(), url.text_range());
                insert_with_ctx(inputs, id_seg.clone(), input, ctx);
            }
            if should_remove_input(change, ctx, &id_seg) {
                return Some(empty_node());
            }
        }

        let context: Context = id_seg.clone().into();
        if walk_input(inputs, &attr, &Some(context), change).is_some() {
            let replacement = remove_child_with_whitespace(nested_attr, &attr, attr.index());
            return Some(replacement);
        }
    }

    None
}

/// Handle a `NODE_IDENT` child during input walking, covering flat-style
/// declarations like `inputs.nixpkgs.url = "..."`.
fn handle_child_ident(
    inputs: &mut HashMap<String, Input>,
    child: &rnix::SyntaxElement,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    let child_node = child.as_node()?;
    let parent_sibling = child_node.parent().and_then(|p| p.next_sibling());

    // `inputs` ident with a sibling (e.g. `inputs.foo.url = ...` or
    // `inputs.foo = { ... }`).
    if child.to_string() == "inputs"
        && let Some(next_sibling) = child_node.next_sibling()
    {
        match next_sibling.kind() {
            SyntaxKind::NODE_IDENT => {
                if let Some(url_id) = next_sibling.next_sibling() {
                    if url_id.kind() == SyntaxKind::NODE_IDENT
                        && let Some(value) = &parent_sibling
                    {
                        if url_id.to_string() == "url" {
                            if let Some(result) =
                                handle_flat_url(inputs, &next_sibling, value, ctx, change)
                            {
                                return Some(result);
                            }
                        } else if url_id.to_string() == "flake"
                            && let Some(result) =
                                handle_flat_flake(&next_sibling, value, ctx, change)
                        {
                            return Some(result);
                        }
                    }
                } else if let Some(nested_attr) = &parent_sibling
                    && let Some(result) =
                        handle_nested_input(inputs, &next_sibling, nested_attr, ctx, change)
                {
                    return Some(result);
                }
            }
            SyntaxKind::NODE_ATTR_SET => {}
            _ => {}
        }
    }

    if child.to_string().starts_with("inputs") {
        let id = child_node.next_sibling()?;
        let context: Context = Segment::from_syntax_or_sentinel(&id).into();
        if walk_inputs(inputs, child_node.clone(), &Some(context), change).is_some() {
            tracing::warn!(
                "Flat tree attribute replacement not yet implemented for: {}",
                child
            );
        }
    }

    None
}

/// Handle a `NODE_ATTRPATH_VALUE` child during input walking.
fn handle_child_attrpath_value(
    inputs: &mut HashMap<String, Input>,
    parent: &SyntaxNode,
    child: &rnix::SyntaxElement,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    let child_node = child.as_node().unwrap();

    // Build a single-segment context from the attrpath when missing.
    let ctx = if ctx.is_none() {
        let maybe_input_id = child_node.children().find_map(|c| {
            c.children()
                .find(|child| child.to_string() == "inputs")
                .and_then(|input_child| input_child.prev_sibling())
        });
        maybe_input_id.map(|id| Segment::from_syntax_or_sentinel(&id).into())
    } else {
        ctx.clone()
    };

    if let Some(replacement) = walk_input(inputs, child_node, &ctx, change) {
        let mut green = parent
            .green()
            .replace_child(child.index(), replacement.green().into());

        if replacement.text().is_empty() {
            let mut to_remove = trailing_inline_comment_indices(child);
            if let Some(ws_index) = adjacent_whitespace_index(child) {
                to_remove.push(ws_index);
            }
            to_remove.sort_unstable();
            for idx in to_remove.into_iter().rev() {
                green = green.remove_child(idx);
            }
        }
        return Some(SyntaxNode::new_root(green));
    }

    // Add a new entry into a non-empty `inputs = { ... }` block.
    if ctx.is_none()
        && let Change::Add {
            id: Some(id),
            uri: Some(uri),
            flake,
        } = change
    {
        return Some(insert_added_input_into_block(
            parent,
            child,
            child_node,
            id.input().as_str(),
            uri,
            *flake,
        ));
    }

    None
}

/// Splice a new `id = ...` entry into a non-empty `inputs = { ... }` block.
///
/// `child` is the iteration cursor that triggered the add; `child_node` is the
/// same node typed as `SyntaxNode`. Both are kept as fallbacks for the
/// degenerate case where `parent` has no `NODE_ATTRPATH_VALUE` children at all:
/// the caller has only verified that `child` itself is one, but the lookup below
/// re-scans `parent.children()` and is paranoid about an empty result.
fn insert_added_input_into_block(
    parent: &SyntaxNode,
    child: &rnix::SyntaxElement,
    child_node: &SyntaxNode,
    id: &str,
    uri: &str,
    flake: bool,
) -> SyntaxNode {
    let last_attr = parent
        .children()
        .filter(|c| c.kind() == SyntaxKind::NODE_ATTRPATH_VALUE)
        .last();
    let insert_index = last_attr
        .as_ref()
        .map(|c| {
            let elem: rnix::SyntaxElement = c.clone().into();
            elem.index() + 1
        })
        .unwrap_or(child.index());

    let use_attrset = uses_attrset_style(parent);

    // Reuse the whitespace before the last input but normalize to a single
    // newline + indent. Copying the raw inter-entry whitespace would
    // duplicate blank lines when the closing brace already has one.
    let ws_reference = last_attr.as_ref().unwrap_or(child_node);
    if let Some(whitespace) = get_sibling_whitespace(ws_reference) {
        let ws_str = whitespace.to_string();
        let ws_node = parse_node(last_line_with_newline(&ws_str));
        let mut green = parent
            .green()
            .insert_child(insert_index, ws_node.green().into());
        let mut offset = 1;

        if use_attrset {
            let indent = extract_indent(&ws_str);
            let uri_node = if flake {
                make_attrset_url_attr(id, uri, indent)
            } else {
                make_attrset_url_flake_false_attr(id, uri, indent)
            };
            green = green.insert_child(insert_index + offset, uri_node.green().into());
        } else {
            let uri_node = make_url_attr(id, uri);
            green = green.insert_child(insert_index + offset, uri_node.green().into());
            offset += 1;

            if !flake {
                let no_flake = make_flake_false_attr(id);
                let compact_ws_node = parse_node(last_line_with_newline(&ws_str));
                green = green.insert_child(insert_index + offset, compact_ws_node.green().into());
                offset += 1;
                green = green.insert_child(insert_index + offset, no_flake.green().into());
            }
        }
        return SyntaxNode::new_root(green);
    }

    let uri_node = make_url_attr(id, uri);
    let mut green = parent
        .green()
        .insert_child(insert_index, uri_node.green().into());

    if !flake {
        let no_flake = make_flake_false_attr(id);
        green = green.insert_child(insert_index + 1, no_flake.green().into());
    }
    SyntaxNode::new_root(green)
}

/// Handle a `NODE_ATTRPATH` whose last segment is `follows`, at any depth.
///
/// The owning input is the first non-`inputs` segment of the attrpath. The
/// remaining non-`inputs` segments (excluding the trailing `follows`) form
/// the nested-input path recorded on the owner as
/// [`crate::input::Follows::Indirect`].
fn handle_attrpath_follows(
    inputs: &mut HashMap<String, Input>,
    node: &SyntaxNode,
    change: &Change,
) -> Option<SyntaxNode> {
    let parts = parse_follows_attrpath_parts(node)?;
    if parts.rest.is_empty() {
        record_depth_one_follows_attr(inputs, &parts.owner_seg, &parts.url_node, change)
    } else {
        record_depth_n_follows_attr(inputs, &parts, change)
    }
}

struct FollowsAttrPathParts {
    /// Kept alongside [`Self::owner_seg`] because the depth-2 remove path
    /// matches against a segment built from the syntax node, not from the
    /// already-derived owner segment.
    owner_node: SyntaxNode,
    owner_seg: Segment,
    rest: Vec<Segment>,
    url_node: SyntaxNode,
}

/// Returns `None` for any non-follows attrpath, for an attrpath that is
/// nothing but `inputs` keywords, or when the binding has no value sibling.
fn parse_follows_attrpath_parts(node: &SyntaxNode) -> Option<FollowsAttrPathParts> {
    let children: Vec<SyntaxNode> = node.children().collect();
    let last = children.last()?;
    if last.to_string() != "follows" {
        return None;
    }

    let path_segments: Vec<(SyntaxNode, Segment)> = children[..children.len() - 1]
        .iter()
        .filter(|c| c.to_string() != "inputs")
        .map(|c| (c.clone(), Segment::from_syntax_or_sentinel(c)))
        .collect();

    let (owner_node, owner_seg) = path_segments.first().cloned()?;
    let rest: Vec<Segment> = path_segments[1..].iter().map(|(_, s)| s.clone()).collect();
    let url_node = node.next_sibling()?;

    Some(FollowsAttrPathParts {
        owner_node,
        owner_seg,
        rest,
        url_node,
    })
}

/// `inputs.<owner>.follows = "T"`. The target text is stored as a synthetic
/// url on the owning input so depth-1 ctx-driven flows downstream
/// (`insert_with_ctx` and the follows-graph build) keep seeing a populated
/// `url` field where they expect one.
fn record_depth_one_follows_attr(
    inputs: &mut HashMap<String, Input>,
    owner_seg: &Segment,
    url_node: &SyntaxNode,
    change: &Change,
) -> Option<SyntaxNode> {
    let input = Input::with_url(
        owner_seg.clone(),
        url_node.to_string(),
        url_node.text_range(),
    );
    insert_with_ctx(inputs, owner_seg.clone(), input, &None);
    if change.is_remove()
        && let Some(id) = change.id()
        && id.matches_with_follows(owner_seg, Some(owner_seg))
    {
        return Some(empty_node());
    }
    None
}

/// `insert_with_ctx`'s single-segment path would truncate intermediate
/// segments at depth ≥ 2, so the indirect edge is built and pushed
/// directly.
fn record_depth_n_follows_attr(
    inputs: &mut HashMap<String, Input>,
    parts: &FollowsAttrPathParts,
    change: &Change,
) -> Option<SyntaxNode> {
    let follows_seg = parts.rest.last().cloned().expect("rest is non-empty");
    let leaf_url = parts.url_node.to_string();
    let target = AttrPath::parse_follows_target(&leaf_url, &follows_seg);

    let mut path_iter = parts.rest.iter().cloned();
    let mut nested_path = AttrPath::new(path_iter.next().expect("rest is non-empty"));
    for seg in path_iter {
        nested_path.push(seg);
    }

    let key = parts.owner_seg.as_str().to_string();
    let entry = inputs
        .entry(key)
        .or_insert_with(|| Input::new(parts.owner_seg.clone()));
    entry.push_indirect_follows(nested_path, target);

    if change.is_remove()
        && let Some(id) = change.id()
        && depth_n_follows_attr_matches_remove(parts, &id, &follows_seg)
    {
        return Some(empty_node());
    }
    None
}

/// Depth-2 uses the two-segment
/// [`crate::change::ChangeId::matches_with_follows`] matcher because
/// [`crate::change::ChangeId::input`] /
/// [`crate::change::ChangeId::follows`] only expose the first two segments
/// of the id. Depth-N has to compare the full id path against the chain.
fn depth_n_follows_attr_matches_remove(
    parts: &FollowsAttrPathParts,
    id: &crate::change::ChangeId,
    follows_seg: &Segment,
) -> bool {
    if parts.rest.len() == 1 {
        let owner_match_seg = Segment::from_syntax_or_sentinel(&parts.owner_node);
        return id.matches_with_follows(&owner_match_seg, Some(follows_seg));
    }
    let id_segs = id.path().segments();
    id_segs.len() == parts.rest.len() + 1
        && id_segs[0] == parts.owner_seg
        && id_segs[1..]
            .iter()
            .zip(parts.rest.iter())
            .all(|(a, b)| a == b)
}

fn handle_url_attr(
    inputs: &mut HashMap<String, Input>,
    node: &SyntaxNode,
    child: &SyntaxNode,
    attr: &SyntaxNode,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    if let Some(result) = apply_flat_url_attr(inputs, node, child, attr, ctx, change) {
        return Some(result);
    }
    record_url_sibling_nested_follows(inputs, child, ctx, change);
    None
}

/// `inputs.<id>.url = "..."` at the inputs-block level. With `ctx` set, the
/// same shape inside another input's attrset is a transitive URL override,
/// not a follows; returning `None` here keeps `insert_with_ctx` from
/// misreading the URL string as a follows target.
fn apply_flat_url_attr(
    inputs: &mut HashMap<String, Input>,
    node: &SyntaxNode,
    child: &SyntaxNode,
    attr: &SyntaxNode,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    let prev_id = attr.prev_sibling()?;
    if ctx.is_some() {
        return None;
    }
    let prev_seg = Segment::from_syntax_or_sentinel(&prev_id);
    let prev_str = prev_seg.as_str().to_string();
    if let Change::Remove { ids } = change
        && ids
            .iter()
            .any(|id| id.input().as_str() == prev_str && id.follows().is_none())
    {
        return Some(empty_node());
    }
    if let Change::Change { id, uri, .. } = change
        && let Some(id) = id
        && id.input().as_str() == prev_str
        && id.follows().is_none()
        && let Some(uri) = uri
        && let Some(url_node) = child.next_sibling()
    {
        let new_url = make_quoted_string(uri);
        return Some(substitute_child(node, url_node.index(), &new_url));
    }
    if let Some(sibling) = child.next_sibling() {
        let input = Input::with_url(prev_seg.clone(), sibling.to_string(), sibling.text_range());
        insert_with_ctx(inputs, prev_seg, input, ctx);
    }
    None
}

fn record_url_sibling_nested_follows(
    inputs: &mut HashMap<String, Input>,
    child: &SyntaxNode,
    ctx: &Option<Context>,
    _change: &Change,
) {
    let Some(parent) = child.parent() else { return };
    let Some(sibling) = parent.next_sibling() else {
        return;
    };
    let Some(nested_child) = sibling.first_child() else {
        return;
    };
    if nested_child.to_string() != "inputs" {
        return;
    }
    let Some(attr_set) = nested_child.next_sibling() else {
        return;
    };
    if attr_set.kind() != SyntaxKind::NODE_ATTR_SET {
        return;
    }

    for nested_attr in attr_set.children() {
        let Some(attrpath) = nested_attr.first_child() else {
            continue;
        };
        let Some(first_ident) = attrpath.first_child() else {
            continue;
        };

        if let Some(follows_ident) = first_ident.next_sibling() {
            if follows_ident.to_string() != "follows" {
                continue;
            }
            record_nested_flat_follows(inputs, &attrpath, &first_ident, ctx);
        } else if let Some(value_node) = attrpath.next_sibling()
            && value_node.kind() == SyntaxKind::NODE_ATTR_SET
        {
            record_nested_attrset_follows(inputs, &first_ident, &value_node, ctx);
        }
    }
}

fn record_nested_flat_follows(
    inputs: &mut HashMap<String, Input>,
    attrpath: &SyntaxNode,
    first_ident: &SyntaxNode,
    ctx: &Option<Context>,
) {
    let id_seg = Segment::from_syntax_or_sentinel(first_ident);
    let Some(follows) = attrpath.next_sibling() else {
        return;
    };
    let input = Input::with_url(id_seg.clone(), follows.to_string(), follows.text_range());
    insert_with_ctx(inputs, id_seg, input, ctx);
}

fn record_nested_attrset_follows(
    inputs: &mut HashMap<String, Input>,
    first_ident: &SyntaxNode,
    value_node: &SyntaxNode,
    ctx: &Option<Context>,
) {
    let id_seg = Segment::from_syntax_or_sentinel(first_ident);
    for inner_attr in value_node.children() {
        let Some(inner_path) = inner_attr.first_child() else {
            continue;
        };
        let Some(inner_ident) = inner_path.first_child() else {
            continue;
        };
        if inner_ident.to_string() != "follows" {
            continue;
        }
        let Some(follows) = inner_path.next_sibling() else {
            continue;
        };
        let input = Input::with_url(id_seg.clone(), follows.to_string(), follows.text_range());
        insert_with_ctx(inputs, id_seg.clone(), input, ctx);
    }
}

/// Handle a `flake = ...` binding inside an input's attrset.
fn handle_flake_attr(
    inputs: &mut HashMap<String, Input>,
    attr: &SyntaxNode,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    if let Some(input_id) = attr.prev_sibling()
        && let Some(is_flake) = attr.parent().unwrap().next_sibling()
    {
        let id_seg = Segment::from_syntax_or_sentinel(&input_id);
        let mut input = Input::new(id_seg.clone());
        input.flake = is_flake.to_string().parse().unwrap();
        let text_range = input_id.text_range();
        input.range = crate::input::Range::from_text_range(text_range);
        insert_with_ctx(inputs, id_seg.clone(), input, ctx);
        if should_remove_nested_input(change, ctx, &id_seg) {
            return Some(empty_node());
        }
    }
    None
}

/// Handle a `follows = ...` binding inside an input's attrset, at any
/// nesting depth.
///
/// Strips the literal `inputs` keywords from the parent attrpath. The
/// surviving idents are the source chain. The owning input comes from `ctx`
/// when present, or from `chain[0]` otherwise.
fn handle_follows_attr(
    inputs: &mut HashMap<String, Input>,
    attr: &SyntaxNode,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    let attrpath = attr.parent().unwrap();
    let follows_value = attrpath.next_sibling().unwrap();

    let chain: Vec<SyntaxNode> = attrpath
        .children()
        .filter(|c| c.to_string() != "inputs" && c.to_string() != "follows")
        .collect();

    if chain.is_empty() {
        return None;
    }

    let (owner_seg, nested_segs) = resolve_follows_owner_and_nested(&attrpath, &chain, ctx);

    if nested_segs.len() <= 1 {
        // Depth-1 stores a single-segment leaf and runs the remove-flow.
        // The depth-N branch below builds a typed multi-segment path.
        let leaf_seg = nested_segs
            .first()
            .cloned()
            .unwrap_or_else(|| owner_seg.clone());
        let input = Input::with_url(
            leaf_seg.clone(),
            follows_value.to_string(),
            follows_value.text_range(),
        );
        insert_with_ctx(inputs, leaf_seg.clone(), input.clone(), ctx);
        if should_remove_input(change, ctx, input.id())
            || should_remove_nested_input(change, ctx, input.id())
        {
            return Some(empty_node());
        }
        return None;
    }

    let mut path_iter = nested_segs.iter().cloned();
    let mut nested_path = AttrPath::new(path_iter.next().expect("len ≥ 2 above"));
    for seg in path_iter {
        nested_path.push(seg);
    }
    let url_text = follows_value.to_string();
    let unquoted = strip_outer_quotes(&url_text);
    let leaf_seg = nested_path.last().clone();
    let target = AttrPath::parse_follows_target(unquoted, &leaf_seg);

    let key = owner_seg.as_str().to_string();
    let entry = inputs
        .entry(key)
        .or_insert_with(|| Input::new(owner_seg.clone()));
    entry.push_indirect_follows(nested_path, target);

    // Depth-N removal: same shape as the depth-N branch in
    // [`handle_attrpath_follows`]. The chain's owner comes from `ctx` when
    // present.
    if change.is_remove()
        && let Some(id) = change.id()
    {
        let id_segs = id.path().segments();
        if id_segs.len() == nested_segs.len() + 1
            && id_segs[0] == owner_seg
            && id_segs[1..]
                .iter()
                .zip(nested_segs.iter())
                .all(|(a, b)| a == b)
        {
            return Some(empty_node());
        }
    }

    None
}

/// Resolve the `(owner, nested)` pair for a follows attrpath.
///
/// Discriminator: the raw attrpath's first ident is `inputs` for the
/// inside-an-input-block shape (chain is the nested path under the
/// ctx-supplied owner). Otherwise chain[0] is treated as the owner ident
/// and stripped from the nested path; a `ctx` that already names the owner
/// still wins the owner slot, but chain handling matches by structural shape,
/// not by ident equality.
///
/// Precondition: `chain` is non-empty.
fn resolve_follows_owner_and_nested(
    attrpath: &SyntaxNode,
    chain: &[SyntaxNode],
    ctx: &Option<Context>,
) -> (Segment, Vec<Segment>) {
    let attrpath_starts_with_inputs = attrpath
        .children()
        .next()
        .map(|c| c.to_string() == "inputs")
        .unwrap_or(false);
    let chain_first = Segment::from_syntax_or_sentinel(&chain[0]);
    match ctx.as_ref().and_then(|c| c.first().cloned()) {
        Some(ctx_owner) if attrpath_starts_with_inputs => (
            ctx_owner,
            chain.iter().map(Segment::from_syntax_or_sentinel).collect(),
        ),
        Some(ctx_owner) if ctx_owner == chain_first => (
            ctx_owner,
            chain[1..]
                .iter()
                .map(Segment::from_syntax_or_sentinel)
                .collect(),
        ),
        Some(ctx_owner) => (
            ctx_owner,
            chain.iter().map(Segment::from_syntax_or_sentinel).collect(),
        ),
        None => (
            chain_first,
            chain[1..]
                .iter()
                .map(Segment::from_syntax_or_sentinel)
                .collect(),
        ),
    }
}

/// Dispatch a `NODE_ATTRPATH` inside an input attrset to the `url`, `flake`,
/// or `follows` handler based on the leaf attribute name.
fn handle_input_attrpath(
    inputs: &mut HashMap<String, Input>,
    node: &SyntaxNode,
    child: &SyntaxNode,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    for attr in child.children() {
        let attr_name = attr.to_string();
        match attr_name.as_str() {
            "url" => {
                if let Some(result) = handle_url_attr(inputs, node, child, &attr, ctx, change) {
                    return Some(result);
                }
            }
            "flake" => {
                if let Some(result) = handle_flake_attr(inputs, &attr, ctx, change) {
                    return Some(result);
                }
            }
            "follows" => {
                if let Some(result) = handle_follows_attr(inputs, &attr, ctx, change) {
                    return Some(result);
                }
            }
            _ => {}
        }
    }
    None
}

/// Result of a follows-attr lookup: the `NODE_ATTRPATH_VALUE` child carrying
/// the match, its value node (if any), and whether the existing target already
/// equals the desired one.
struct ExistingFollows {
    attr: SyntaxNode,
    value: Option<SyntaxNode>,
    same_target: bool,
}

/// Locate a follows attr inside `container` whose attrpath idents (with outer
/// `"..."` stripped) match `expected` pairwise.
///
/// `expected` covers the entire attrpath, e.g. `["inputs", "nixpkgs", "follows"]`
/// or `["crane", "inputs", "nixpkgs", "follows"]`. The caller decides how to
/// splice the replacement back into the surrounding tree.
fn find_existing_follows(
    container: &SyntaxNode,
    expected: &[&str],
    target: &str,
) -> Option<ExistingFollows> {
    for attr in container.children() {
        if attr.kind() != SyntaxKind::NODE_ATTRPATH_VALUE {
            continue;
        }
        let Some(attrpath) = attr
            .children()
            .find(|c| c.kind() == SyntaxKind::NODE_ATTRPATH)
        else {
            continue;
        };
        let idents: Vec<String> = attrpath.children().map(|c| c.to_string()).collect();
        if idents.len() != expected.len() {
            continue;
        }
        if !idents
            .iter()
            .zip(expected.iter())
            .all(|(have, want)| strip_outer_quotes(have) == *want)
        {
            continue;
        }
        let value = attrpath.next_sibling();
        let current_target = value
            .as_ref()
            .map(|v| strip_outer_quotes(&v.to_string()).to_string())
            .unwrap_or_default();
        return Some(ExistingFollows {
            attr,
            value,
            same_target: current_target == target,
        });
    }
    None
}

/// Locate an `inputs.<R0>...inputs.<RN>.follows` attr inside an input's
/// `{ ... }` block and rebuild the surrounding `node`.
///
/// Returns the unchanged outer node on a same-target hit, the retargeted
/// outer node on a different-target hit, or `None` when no match exists.
fn find_existing_nested_follows(
    node: &SyntaxNode,
    attr_set: &SyntaxNode,
    rest: &[Segment],
    target: &str,
) -> Option<Option<SyntaxNode>> {
    let expected = follows_idents_prefixed(rest);
    let found = find_existing_follows(attr_set, &expected, target)?;
    if found.same_target {
        return Some(Some(node.clone()));
    }
    let value = found.value?;
    let new_value = make_quoted_string(target);
    let new_attr = substitute_child(&found.attr, value.index(), &new_value);
    let new_child = substitute_child(attr_set, found.attr.index(), &new_attr);
    Some(Some(substitute_child(node, attr_set.index(), &new_child)))
}

/// Locate a flat `<S0>.inputs.<S1>...inputs.<SN>.follows` attr at `node`'s
/// level (e.g. inside an `inputs = { ... }` block) and rebuild it the same
/// way as [`find_existing_nested_follows`].
fn find_existing_flat_follows(
    node: &SyntaxNode,
    path: &[Segment],
    target: &str,
) -> Option<Option<SyntaxNode>> {
    let expected = follows_idents_bare(path);
    let found = find_existing_follows(node, &expected, target)?;
    if found.same_target {
        return Some(Some(node.clone()));
    }
    let value = found.value?;
    let new_value = make_quoted_string(target);
    let new_attr = substitute_child(&found.attr, value.index(), &new_value);
    Some(Some(substitute_child(node, found.attr.index(), &new_attr)))
}

fn handle_url_leaf(
    inputs: &mut HashMap<String, Input>,
    node: &SyntaxNode,
    child: &SyntaxNode,
    attr: &SyntaxNode,
    leaf: &SyntaxNode,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    let id_node = child.prev_sibling().unwrap();
    let id_seg = Segment::from_syntax_or_sentinel(&id_node);
    let id_str = id_seg.as_str().to_string();
    let uri = leaf.next_sibling().unwrap();
    let input = Input::with_url(id_seg.clone(), uri.to_string(), uri.text_range());
    insert_with_ctx(inputs, id_seg.clone(), input, ctx);

    if let Change::Remove { ids } = change
        && ids
            .iter()
            .any(|candidate| candidate.input().as_str() == id_str && candidate.follows().is_none())
    {
        return Some(empty_node());
    }

    if let Change::Change {
        id: Some(change_id),
        uri: Some(new_uri),
        ..
    } = change
        && change_id.input().as_str() == id_str
        && change_id.follows().is_none()
    {
        let new_url = make_quoted_string(new_uri);
        let new_attr = substitute_child(attr, leaf.next_sibling().unwrap().index(), &new_url);
        let new_child = substitute_child(child, attr.index(), &new_attr);
        return Some(substitute_child(node, child.index(), &new_child));
    }

    None
}

/// The bare `inputs = { ... }` shape needs its own removal-with-pruning
/// path: recursing through [`walk_inputs`] strips the matching entry
/// inside the nested attrset, but cannot prune the now-empty `inputs`
/// binding from the outer input block.
fn handle_inputs_leaf(
    inputs: &mut HashMap<String, Input>,
    node: &SyntaxNode,
    child: &SyntaxNode,
    attr: &SyntaxNode,
    leaf: &SyntaxNode,
    change: &Change,
) -> Option<SyntaxNode> {
    let id_node = child.prev_sibling().unwrap();
    let id_seg = Segment::from_syntax_or_sentinel(&id_node);
    let context: Context = id_seg.clone().into();
    let ctx_some = Some(context);
    if let Some(replacement) = walk_inputs(inputs, child.clone(), &ctx_some, change) {
        return Some(substitute_child(node, child.index(), &replacement));
    }

    if leaf.to_string() == "inputs"
        && change.is_remove()
        && let Some(inputs_attrset) = attr
            .children()
            .find(|c| c.kind() == SyntaxKind::NODE_ATTR_SET)
    {
        for nested_entry in inputs_attrset.children() {
            if nested_entry.kind() != SyntaxKind::NODE_ATTRPATH_VALUE {
                continue;
            }
            let Some(nested_path) = nested_entry.first_child() else {
                continue;
            };
            let Some(nested_id) = nested_path.first_child() else {
                continue;
            };
            let nested_seg = Segment::from_syntax_or_sentinel(&nested_id);
            if should_remove_nested_input(change, &ctx_some, &nested_seg) {
                let new_inputs_attrset = remove_child_with_whitespace(
                    &inputs_attrset,
                    &nested_entry,
                    nested_entry.index(),
                );

                // Comments inside the block count as user-authored content
                // and suppress pruning; only bindings count toward
                // emptiness.
                let new_child = if is_attrset_content_empty(&new_inputs_attrset) {
                    remove_child_with_whitespace(child, attr, attr.index())
                } else {
                    let new_attr =
                        substitute_child(attr, inputs_attrset.index(), &new_inputs_attrset);
                    substitute_child(child, attr.index(), &new_attr)
                };

                return Some(substitute_child(node, child.index(), &new_child));
            }
        }
    }

    None
}

fn find_inputs_block_attr(parent: &SyntaxNode) -> Option<SyntaxNode> {
    parent.children().find(|c| {
        if c.kind() != SyntaxKind::NODE_ATTRPATH_VALUE {
            return false;
        }
        let is_inputs = c
            .first_child()
            .map(|attrpath| attrpath.to_string() == "inputs")
            .unwrap_or(false);
        is_inputs && c.children().any(|v| v.kind() == SyntaxKind::NODE_ATTR_SET)
    })
}

fn merge_follow_into_inputs_block(
    node: &SyntaxNode,
    child: &SyntaxNode,
    rest: &[Segment],
    target: &str,
) -> Option<SyntaxNode> {
    let inputs_attr = find_inputs_block_attr(child)?;
    let inputs_block = inputs_attr
        .children()
        .find(|c| c.kind() == SyntaxKind::NODE_ATTR_SET)?;

    if let Some(maybe_node) = find_existing_flat_follows(&inputs_block, rest, target) {
        let new_block = maybe_node?;
        let new_attr = substitute_child(&inputs_attr, inputs_block.index(), &new_block);
        let new_child = substitute_child(child, inputs_attr.index(), &new_attr);
        return Some(substitute_child(node, child.index(), &new_child));
    }

    let mut path = AttrPath::new(rest[0].clone());
    for seg in &rest[1..] {
        path.push(seg.clone());
    }
    let follows_node = FollowsKind::InputsBlockNested {
        path: &path,
        target,
    }
    .emit();

    let new_block = if let Some(last_attr) = inputs_block
        .children()
        .filter(|c| c.kind() == SyntaxKind::NODE_ATTRPATH_VALUE)
        .last()
    {
        let insert_index = insertion_index_after(&last_attr);
        let mut green = inputs_block
            .green()
            .insert_child(insert_index, follows_node.green().into());
        if let Some(whitespace) = get_sibling_whitespace(&last_attr) {
            let ws_str = whitespace.to_string();
            let ws_node = parse_node(last_line_with_newline(&ws_str));
            green = green.insert_child(insert_index, ws_node.green().into());
        }
        SyntaxNode::new_root(green)
    } else {
        fill_empty_inputs_block(&inputs_attr, &inputs_block, &follows_node)
    };

    let new_attr = substitute_child(&inputs_attr, inputs_block.index(), &new_block);
    let new_child = substitute_child(child, inputs_attr.index(), &new_attr);
    Some(substitute_child(node, child.index(), &new_child))
}

fn fill_empty_inputs_block(
    inputs_attr: &SyntaxNode,
    inputs_block: &SyntaxNode,
    follows_node: &SyntaxNode,
) -> SyntaxNode {
    let parent_attr_indent = inputs_attr
        .prev_sibling_or_token()
        .filter(|t| t.kind() == SyntaxKind::TOKEN_WHITESPACE)
        .map(|t| extract_indent(&t.to_string()).to_string())
        .unwrap_or_else(|| "    ".to_string());
    let entry_indent = format!("\n{parent_attr_indent}  ");
    let closing_indent = format!("\n{parent_attr_indent}");

    // In a comment-only body the leading whitespace is the indent before the
    // first comment, not the filler between an empty `{ }`. The truly-empty
    // path below strips that whitespace, which here would fuse the opening
    // brace onto the comment line. Keep every token and insert just before
    // the closing brace instead.
    if inputs_block
        .children_with_tokens()
        .any(|t| t.kind() == SyntaxKind::TOKEN_COMMENT)
    {
        let mut green = inputs_block.green().into_owned();
        let brace_index = green
            .children()
            .position(|c| c.as_token().map(|t| t.text() == "}").unwrap_or(false))
            .unwrap_or(green.children().count());
        let trailing_ws = brace_index > 0
            && green.children().nth(brace_index - 1).is_some_and(|c| {
                c.as_token()
                    .is_some_and(|t| !t.text().is_empty() && t.text().trim().is_empty())
            });
        let insert_index = if trailing_ws {
            brace_index - 1
        } else {
            brace_index
        };
        green = green.insert_child(insert_index, follows_node.green().into());
        green = green.insert_child(insert_index, parse_node(&entry_indent).green().into());
        return SyntaxNode::new_root(green);
    }

    let ws_index = inputs_block
        .children_with_tokens()
        .find(|t| t.kind() == SyntaxKind::TOKEN_WHITESPACE)
        .map(|t| t.index());
    let mut green = if let Some(idx) = ws_index {
        inputs_block.green().remove_child(idx)
    } else {
        inputs_block.green().into_owned()
    };

    let brace_index = green
        .children()
        .position(|c| c.as_token().map(|t| t.text() == "}").unwrap_or(false))
        .unwrap_or(green.children().count());

    green = green.insert_child(brace_index, parse_node(&closing_indent).green().into());
    green = green.insert_child(brace_index, follows_node.green().into());
    green = green.insert_child(brace_index, parse_node(&entry_indent).green().into());

    SyntaxNode::new_root(green)
}

fn handle_input_attr_set(
    inputs: &mut HashMap<String, Input>,
    node: &SyntaxNode,
    child: &SyntaxNode,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    for attr in child.children() {
        for leaf in attr.children() {
            let leaf_text = leaf.to_string();
            if leaf_text == "url"
                && let Some(result) =
                    handle_url_leaf(inputs, node, child, &attr, &leaf, ctx, change)
            {
                return Some(result);
            }

            if leaf_text.starts_with("inputs")
                && let Some(result) = handle_inputs_leaf(inputs, node, child, &attr, &leaf, change)
            {
                return Some(result);
            }
        }
    }

    if let Change::Follows { input, target } = change {
        let full_path = input.path();
        let parent_id = input.input();
        let parent_id_str = parent_id.as_str();
        let target_str = target.to_flake_follows_string();

        if let Some(id_node) = child.prev_sibling()
            && strip_outer_quotes(&id_node.to_string()) == parent_id_str
        {
            // Inside the parent's `{ ... }` block, emit the chain relative
            // to the parent (everything below it).
            let rest: Vec<Segment> = full_path.segments()[1..].to_vec();
            if !rest.is_empty() {
                if let Some(result) =
                    merge_follow_into_inputs_block(node, child, &rest, &target_str)
                {
                    return Some(result);
                }

                if let Some(result) = find_existing_nested_follows(node, child, &rest, &target_str)
                {
                    return result;
                }

                let follows_node = FollowsKind::BlockNested {
                    rest: &rest,
                    target: &target_str,
                }
                .emit();

                let children: Vec<_> = child.children().collect();
                if let Some(last_child) = children.last() {
                    let insert_index = insertion_index_after(last_child);

                    let mut green = child
                        .green()
                        .insert_child(insert_index, follows_node.green().into());

                    if let Some(whitespace) = get_sibling_whitespace(last_child) {
                        green = green.insert_child(insert_index, whitespace.green().into());
                    }

                    let new_child = SyntaxNode::new_root(green);
                    return Some(substitute_child(node, child.index(), &new_child));
                }
            } else if full_path.len() == 1 {
                let has_follows = child.children().any(|attr| {
                    attr.first_child()
                        .and_then(|attrpath| attrpath.first_child())
                        .map(|first_ident| first_ident.to_string() == "follows")
                        .unwrap_or(false)
                });

                if !has_follows {
                    let follows_node = FollowsKind::BlockBare {
                        target: &target_str,
                    }
                    .emit();
                    let children: Vec<_> = child.children().collect();
                    if let Some(last_child) = children.last() {
                        let insert_index = insertion_index_after(last_child);
                        let mut green = child
                            .green()
                            .insert_child(insert_index, follows_node.green().into());

                        if let Some(whitespace) = get_sibling_whitespace(last_child) {
                            green = green.insert_child(insert_index, whitespace.green().into());
                        }

                        let new_child = SyntaxNode::new_root(green);
                        return Some(substitute_child(node, child.index(), &new_child));
                    }
                }
            }
        }
    }

    None
}

/// Walk a single input declaration in either flat or attrset shape:
///
/// ```nix
/// flake-utils.url = "github:numtide/flake-utils";
/// ```
///
/// or
///
/// ```nix
/// rust-overlay = {
///   url = "github:oxalica/rust-overlay";
///   inputs.nixpkgs.follows = "nixpkgs";
///   inputs.flake-utils.follows = "flake-utils";
/// };
/// ```
pub(crate) fn walk_input(
    inputs: &mut HashMap<String, Input>,
    node: &SyntaxNode,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    for child in node.children() {
        if child.kind() == SyntaxKind::NODE_ATTRPATH
            && let Some(result) = handle_input_attrpath(inputs, node, &child, ctx, change)
        {
            return Some(result);
        }

        if child.kind() == SyntaxKind::NODE_ATTR_SET
            && let Some(result) = handle_input_attr_set(inputs, node, &child, ctx, change)
        {
            return Some(result);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use rnix::{Root, SyntaxKind, SyntaxNode};

    use super::{
        apply_add, apply_change_uri, apply_follows, apply_remove, handle_inputs_leaf,
        handle_url_leaf, insert_added_input_into_block, resolve_follows_owner_and_nested,
    };
    use crate::change::{Change, ChangeId};
    use crate::follows::{AttrPath, Segment};
    use crate::walk::Walker;
    use crate::walk::context::Context;

    /// Locate the `inputs = { ... }` value attrset inside a parsed flake. The
    /// returned node is the right-hand side `NODE_ATTR_SET`, ready to feed into
    /// the per-variant apply functions.
    fn parse_inputs_block(flake: &str) -> SyntaxNode {
        let root = Root::parse(flake).syntax();
        fn find(node: &SyntaxNode) -> Option<SyntaxNode> {
            for child in node.children() {
                if child.kind() == SyntaxKind::NODE_ATTRPATH_VALUE
                    && let Some(attrpath) = child.first_child()
                    && let Some(first) = attrpath.first_child()
                    && first.to_string() == "inputs"
                    && let Some(value) = attrpath.next_sibling()
                    && value.kind() == SyntaxKind::NODE_ATTR_SET
                {
                    return Some(value);
                }
                if let Some(found) = find(&child) {
                    return Some(found);
                }
            }
            None
        }
        find(&root).expect("inputs = { ... } block not found")
    }

    /// Apply `change` to `flake_text` via [`Walker`] and return the resulting
    /// flake.nix text. Asserts the walker actually rewrites the tree.
    fn apply(flake_text: &str, change: &Change) -> String {
        let mut walker = Walker::new(flake_text);
        let result = walker
            .walk(change)
            .expect("walker error")
            .expect("walker did not rewrite the tree");
        result.to_string()
    }

    /// Like [`apply`], but tolerates the walker returning `None` (no rewrite).
    fn apply_maybe(flake_text: &str, change: &Change) -> Option<String> {
        let mut walker = Walker::new(flake_text);
        walker
            .walk(change)
            .expect("walker error")
            .map(|n| n.to_string())
    }

    /// Re-run the walker until it converges, mirroring the loop in
    /// [`FlakeEdit::apply_change`] for `Change::Remove`.
    fn apply_until_fixed(flake_text: &str, change: &Change) -> String {
        let mut walker = Walker::new(flake_text);
        let mut last: Option<String> = None;
        // Hard cap guards against a future non-monotonic regression in
        // walker passes. `Change::Remove` is monotonic today, but the
        // helper has no semantic stake in that.
        for _ in 0..16 {
            let Some(changed) = walker.walk(change).expect("walker error") else {
                break;
            };
            let s = changed.to_string();
            if last.as_ref() == Some(&s) {
                break;
            }
            last = Some(s);
            walker.root = changed;
        }
        last.expect("walker did not rewrite the tree")
    }

    #[test]
    fn remove_depth_two_follows_inputs_block_flat() {
        let flake = r#"{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-edit.url = "github:a-kenji/flake-edit";
    flake-edit.inputs.nixpkgs.follows = "nixpkgs";
    flake-edit.inputs.nested-helper.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, ... }: { };
}
"#;
        let change = Change::Remove {
            ids: vec![ChangeId::parse("flake-edit.nested-helper.nixpkgs").unwrap()],
        };
        let result = apply(flake, &change);

        assert!(
            !result.contains("nested-helper"),
            "depth-2 follows line should be removed, got:\n{result}"
        );
        assert!(
            result.contains("flake-edit.url ="),
            "depth-1 url declaration must remain intact, got:\n{result}"
        );
        assert!(
            result.contains("flake-edit.inputs.nixpkgs.follows = \"nixpkgs\""),
            "depth-1 follows must remain intact, got:\n{result}"
        );
    }

    #[test]
    fn remove_depth_two_follows_block_style() {
        let flake = r#"{
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  inputs.flake-edit = {
    url = "github:a-kenji/flake-edit";
    inputs.nixpkgs.follows = "nixpkgs";
    inputs.nested-helper.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, ... }: { };
}
"#;
        let change = Change::Remove {
            ids: vec![ChangeId::parse("flake-edit.nested-helper.nixpkgs").unwrap()],
        };
        let result = apply(flake, &change);

        assert!(
            !result.contains("nested-helper"),
            "depth-2 follows line should be removed, got:\n{result}"
        );
        assert!(
            result.contains("url = \"github:a-kenji/flake-edit\""),
            "parent input's url binding must remain intact, got:\n{result}"
        );
        assert!(
            result.contains("inputs.nixpkgs.follows = \"nixpkgs\""),
            "depth-1 follows in parent block must remain intact, got:\n{result}"
        );
    }

    #[test]
    fn remove_depth_three_follows() {
        let flake = r#"{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    omnibus.url = "github:Lehmanator/nix-configs";
    omnibus.inputs.nixpkgs.follows = "nixpkgs";
    omnibus.inputs.flops.inputs.POP.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, ... }: { };
}
"#;
        let change = Change::Remove {
            ids: vec![ChangeId::parse("omnibus.flops.POP.nixpkgs").unwrap()],
        };
        let result = apply(flake, &change);

        assert!(
            !result.contains("flops"),
            "depth-3 follows line should be removed, got:\n{result}"
        );
        assert!(
            result.contains("omnibus.url ="),
            "parent url declaration must remain intact, got:\n{result}"
        );
        assert!(
            result.contains("omnibus.inputs.nixpkgs.follows = \"nixpkgs\""),
            "depth-1 follows must remain intact, got:\n{result}"
        );
    }

    #[test]
    fn remove_nested_follows_prunes_empty_intermediate_block() {
        // The depth-1 follows is the only entry in `disko.inputs = { ... }`.
        // After removal, the now-empty `inputs = { }` block must be pruned
        // along with the line that hosted it.
        let flake = r#"{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";

    disko = {
      url = "github:nix-community/disko";
      inputs = {
        nixpkgs.follows = "nixpkgs";
      };
    };
  };

  outputs = _: { };
}
"#;
        let change = Change::Remove {
            ids: vec![ChangeId::parse("disko.nixpkgs").unwrap()],
        };
        let result = apply_until_fixed(flake, &change);

        assert!(
            !result.contains("inputs = {\n      };"),
            "empty intermediate `inputs = {{ }}` block must be pruned, got:\n{result}"
        );
        assert!(
            !result.contains("nixpkgs.follows"),
            "follows line must be removed, got:\n{result}"
        );
    }

    #[test]
    fn remove_nested_follows_preserves_comment_inside_block() {
        // A user-authored comment inside the nested block is content and
        // must suppress pruning even after the only binding is removed.
        let flake = r#"{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";

    disko = {
      url = "github:nix-community/disko";
      inputs = {
        # keep this annotation
        nixpkgs.follows = "nixpkgs";
      };
    };
  };

  outputs = _: { };
}
"#;
        let change = Change::Remove {
            ids: vec![ChangeId::parse("disko.nixpkgs").unwrap()],
        };
        let result = apply_until_fixed(flake, &change);

        assert!(
            result.contains("# keep this annotation"),
            "user comment inside the inputs block must be preserved, got:\n{result}"
        );
        assert!(
            result.contains("inputs = {"),
            "block carrying a comment must NOT be pruned, got:\n{result}"
        );
    }

    #[test]
    fn pre_existing_empty_inputs_block_is_not_touched() {
        // The user wrote an empty `inputs = { };` deliberately on `disko`,
        // and a separate sibling `other` carries a follows that *will* be
        // removed. The prune branch fires while processing `other`, so
        // the walker actually traverses the tree; the user-authored empty
        // block on `disko` must remain intact. This guards against the
        // prune over-firing on pre-existing empties that were not a
        // consequence of the current removal.
        let flake = r#"{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";

    disko = {
      url = "github:nix-community/disko";
      inputs = {
      };
    };

    other = {
      url = "github:owner/other";
      inputs = {
        nixpkgs = {
          follows = "nixpkgs";
        };
      };
    };
  };

  outputs = _: { };
}
"#;
        let change = Change::Remove {
            ids: vec![ChangeId::parse("other.nixpkgs").unwrap()],
        };
        let result = apply_until_fixed(flake, &change);

        assert!(
            result.contains("url = \"github:nix-community/disko\""),
            "disko url must remain, got:\n{result}"
        );
        // The `disko.inputs = { };` user-authored empty must persist
        // (whitespace-aware, since indentation may shift slightly).
        assert!(
            result.matches("inputs = {").count() >= 2,
            "pre-existing user-authored empty `inputs = {{ }}` on disko must remain alongside the toplevel block, got:\n{result}"
        );
    }

    #[test]
    fn remove_nonexistent_depth_two_follows_does_not_remove_sibling_url() {
        // The depth-2 path is absent from this flake. The depth-N matcher
        // must not fall through to the input-removal handlers and strip
        // the parent `flake-edit.url` line.
        let flake = r#"{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-edit.url = "github:a-kenji/flake-edit";
    flake-edit.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, ... }: { };
}
"#;
        let change = Change::Remove {
            ids: vec![ChangeId::parse("flake-edit.nested-helper.nixpkgs").unwrap()],
        };
        let result = apply_maybe(flake, &change).unwrap_or_else(|| flake.to_string());

        assert!(
            result.contains("flake-edit.url = \"github:a-kenji/flake-edit\""),
            "sibling url line must NOT be removed for a nonexistent depth-2 path, got:\n{result}"
        );
        assert!(
            result.contains("flake-edit.inputs.nixpkgs.follows = \"nixpkgs\""),
            "sibling depth-1 follows line must NOT be removed, got:\n{result}"
        );
        assert!(
            result.contains("nixpkgs.url ="),
            "top-level nixpkgs.url must remain intact, got:\n{result}"
        );
    }

    #[test]
    fn apply_remove_strips_matching_flat_url() {
        let flake = r#"{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    other.url = "github:owner/other";
  };

  outputs = { self, ... }: { };
}
"#;
        let inputs_block = parse_inputs_block(flake);
        let mut map = HashMap::new();
        let change = Change::Remove {
            ids: vec![ChangeId::parse("other").unwrap()],
        };
        let result = apply_remove(&mut map, &inputs_block, &None, &change)
            .expect("apply_remove must rewrite the tree");
        let text = result.to_string();
        assert!(!text.contains("other.url"), "got:\n{text}");
        assert!(text.contains("nixpkgs.url"), "got:\n{text}");
    }

    #[test]
    fn apply_change_uri_rewrites_flat_url() {
        let flake = r#"{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, ... }: { };
}
"#;
        let inputs_block = parse_inputs_block(flake);
        let mut map = HashMap::new();
        let change = Change::Change {
            id: Some(ChangeId::parse("nixpkgs").unwrap()),
            uri: Some("github:NixOS/nixpkgs/nixos-23.11".to_string()),
        };
        let result = apply_change_uri(&mut map, &inputs_block, &None, &change)
            .expect("apply_change_uri must rewrite the tree");
        let text = result.to_string();
        assert!(text.contains("nixos-23.11"), "got:\n{text}");
        assert!(!text.contains("nixos-unstable"), "got:\n{text}");
    }

    #[test]
    fn apply_add_inserts_into_empty_inputs_block() {
        let flake = r#"{
  inputs = { };

  outputs = { self, ... }: { };
}
"#;
        let inputs_block = parse_inputs_block(flake);
        let mut map = HashMap::new();
        let change = Change::Add {
            id: Some(ChangeId::parse("nixpkgs").unwrap()),
            uri: Some("github:NixOS/nixpkgs/nixos-unstable".to_string()),
            flake: true,
        };
        let result = apply_add(&mut map, inputs_block, &None, &change)
            .expect("apply_add must rewrite the tree");
        let text = result.to_string();
        assert!(
            text.contains("nixpkgs.url = \"github:NixOS/nixpkgs/nixos-unstable\""),
            "got:\n{text}"
        );
    }

    #[test]
    fn apply_add_inserts_into_nonempty_inputs_block() {
        let flake = r#"{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, ... }: { };
}
"#;
        let inputs_block = parse_inputs_block(flake);
        let mut map = HashMap::new();
        let change = Change::Add {
            id: Some(ChangeId::parse("flake-utils").unwrap()),
            uri: Some("github:numtide/flake-utils".to_string()),
            flake: true,
        };
        let result = apply_add(&mut map, inputs_block, &None, &change)
            .expect("apply_add must rewrite the tree");
        let text = result.to_string();
        assert!(text.contains("nixpkgs.url ="), "got:\n{text}");
        assert!(
            text.contains("flake-utils.url = \"github:numtide/flake-utils\""),
            "got:\n{text}"
        );
    }

    #[test]
    fn apply_follows_inserts_flat_block_nested() {
        let flake = r#"{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-edit.url = "github:a-kenji/flake-edit";
  };

  outputs = { self, ... }: { };
}
"#;
        let inputs_block = parse_inputs_block(flake);
        let mut map = HashMap::new();
        // Pre-seed the parent_exists check.
        map.insert(
            "flake-edit".to_string(),
            crate::input::Input::new(crate::follows::Segment::from_unquoted("flake-edit").unwrap()),
        );
        let change = Change::Follows {
            input: ChangeId::parse("flake-edit.nixpkgs").unwrap(),
            target: AttrPath::parse("nixpkgs").unwrap(),
        };
        let result = apply_follows(&mut map, inputs_block, &None, &change)
            .expect("apply_follows must rewrite the tree");
        let text = result.to_string();
        assert!(
            text.contains("flake-edit.inputs.nixpkgs.follows = \"nixpkgs\""),
            "got:\n{text}"
        );
    }

    #[test]
    fn apply_follows_into_comment_only_inputs_block_keeps_brace_on_own_line() {
        let flake = r#"{
  inputs = {
    foo = {
      url = "github:owner/foo";
      inputs = {
        # nixpkgs.follows = "nixpkgs";
      };
    };
  };

  outputs = { self, ... }: { };
}
"#;
        let change = Change::Follows {
            input: ChangeId::parse("foo.nixpkgs").unwrap(),
            target: AttrPath::parse("nixpkgs").unwrap(),
        };
        let expected = r#"{
  inputs = {
    foo = {
      url = "github:owner/foo";
      inputs = {
        # nixpkgs.follows = "nixpkgs";
        nixpkgs.follows = "nixpkgs";
      };
    };
  };

  outputs = { self, ... }: { };
}
"#;
        assert_eq!(apply(flake, &change), expected);
    }

    #[test]
    fn apply_follows_inserts_top_level_flat_for_single_segment_input() {
        // Single-segment ChangeId hits the `full_path.len() == 1` branch of
        // [`insert_flat_toplevel_follows`], which emits `FollowsKind::TopLevelFlat`
        // (`inputs.<id>.follows = "<target>"`) and splices it after the matching
        // flat declaration. This is the second of the two shapes apply_follows
        // handles directly (the first is the `>= 2` BlockNested branch).
        let flake = r#"{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-edit.url = "github:a-kenji/flake-edit";
  };

  outputs = { self, ... }: { };
}
"#;
        let inputs_block = parse_inputs_block(flake);
        let mut map = HashMap::new();
        let change = Change::Follows {
            input: ChangeId::parse("flake-edit").unwrap(),
            target: AttrPath::parse("nixpkgs").unwrap(),
        };
        let result = apply_follows(&mut map, inputs_block, &None, &change)
            .expect("apply_follows must rewrite the tree");
        let text = result.to_string();
        assert!(
            text.contains("inputs.flake-edit.follows = \"nixpkgs\""),
            "got:\n{text}"
        );
        // Both original entries must remain.
        assert!(text.contains("nixpkgs.url ="), "got:\n{text}");
        assert!(text.contains("flake-edit.url ="), "got:\n{text}");
    }

    /// Locate the four CST nodes [`handle_url_leaf`] / [`handle_inputs_leaf`]
    /// expect, returned in `(node, child, attr, leaf)` order:
    ///
    /// * `node`: the input's `NODE_ATTRPATH_VALUE` (`<id> = { ... };`).
    /// * `child`: its `NODE_ATTR_SET` value (`{ url = ...; ... }`).
    /// * `attr`: a `NODE_ATTRPATH_VALUE` inside `child`.
    /// * `leaf`: the leading `NODE_ATTRPATH` inside `attr`, whose text
    ///   starts with `leaf_prefix` (`"url"`, `"inputs"`, ...).
    fn find_input_attrset_leaf(
        flake: &str,
        input_id: &str,
        leaf_prefix: &str,
    ) -> (SyntaxNode, SyntaxNode, SyntaxNode, SyntaxNode) {
        let inputs_block = parse_inputs_block(flake);
        for input_node in inputs_block.children() {
            if input_node.kind() != SyntaxKind::NODE_ATTRPATH_VALUE {
                continue;
            }
            let Some(attrpath) = input_node.first_child() else {
                continue;
            };
            let id = attrpath
                .first_child()
                .map(|n| n.to_string())
                .unwrap_or_default();
            if id != input_id {
                continue;
            }
            let Some(value) = attrpath.next_sibling() else {
                continue;
            };
            if value.kind() != SyntaxKind::NODE_ATTR_SET {
                continue;
            }
            for attr in value.children() {
                if attr.kind() != SyntaxKind::NODE_ATTRPATH_VALUE {
                    continue;
                }
                for leaf in attr.children() {
                    if leaf.to_string().starts_with(leaf_prefix) {
                        return (input_node, value, attr, leaf);
                    }
                }
            }
        }
        panic!("input '{input_id}' with attrset leaf '{leaf_prefix}' not found");
    }

    #[test]
    fn handle_url_leaf_records_input_for_change_none() {
        let flake = r#"{
  inputs = {
    nixpkgs = { url = "github:NixOS/nixpkgs/nixos-unstable"; };
  };

  outputs = { self, ... }: { };
}
"#;
        let (node, child, attr, leaf) = find_input_attrset_leaf(flake, "nixpkgs", "url");
        let mut map = HashMap::new();
        let result = handle_url_leaf(&mut map, &node, &child, &attr, &leaf, &None, &Change::None);
        assert!(result.is_none(), "Change::None must not rewrite");
        assert!(map.contains_key("nixpkgs"), "input should be captured");
    }

    #[test]
    fn handle_url_leaf_returns_empty_for_matching_remove() {
        let flake = r#"{
  inputs = {
    nixpkgs = { url = "github:NixOS/nixpkgs/nixos-unstable"; };
  };

  outputs = { self, ... }: { };
}
"#;
        let (node, child, attr, leaf) = find_input_attrset_leaf(flake, "nixpkgs", "url");
        let mut map = HashMap::new();
        let change = Change::Remove {
            ids: vec![ChangeId::parse("nixpkgs").unwrap()],
        };
        let result = handle_url_leaf(&mut map, &node, &child, &attr, &leaf, &None, &change)
            .expect("matching Change::Remove must rewrite");
        assert_eq!(
            result.to_string(),
            "",
            "matching remove should return the empty placeholder node",
        );
    }

    #[test]
    fn handle_url_leaf_rewrites_uri_for_matching_change() {
        let flake = r#"{
  inputs = {
    nixpkgs = { url = "github:NixOS/nixpkgs/nixos-unstable"; };
  };

  outputs = { self, ... }: { };
}
"#;
        let (node, child, attr, leaf) = find_input_attrset_leaf(flake, "nixpkgs", "url");
        let mut map = HashMap::new();
        let change = Change::Change {
            id: Some(ChangeId::parse("nixpkgs").unwrap()),
            uri: Some("github:NixOS/nixpkgs/nixos-23.11".to_string()),
        };
        let result = handle_url_leaf(&mut map, &node, &child, &attr, &leaf, &None, &change)
            .expect("matching Change::Change must rewrite");
        let text = result.to_string();
        assert!(text.contains("nixos-23.11"), "got:\n{text}");
        assert!(!text.contains("nixos-unstable"), "got:\n{text}");
    }

    #[test]
    fn handle_inputs_leaf_recurses_for_nested_remove() {
        let flake = r#"{
  inputs = {
    flake-edit = {
      url = "github:a-kenji/flake-edit";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, ... }: { };
}
"#;
        let (node, child, attr, leaf) = find_input_attrset_leaf(flake, "flake-edit", "inputs");
        let mut map = HashMap::new();
        let change = Change::Remove {
            ids: vec![ChangeId::parse("flake-edit.nixpkgs").unwrap()],
        };
        let result = handle_inputs_leaf(&mut map, &node, &child, &attr, &leaf, &change)
            .expect("nested follows removal must rewrite");
        assert!(
            !result.to_string().contains("inputs.nixpkgs.follows"),
            "nested follow should be gone, got:\n{}",
            result
        );
    }

    #[test]
    fn handle_inputs_leaf_returns_none_for_nonmatching_remove() {
        // Removal target unrelated to anything in this input must leave
        // the recursion empty-handed and fall through to None, since the
        // dispatcher relies on a None return to keep iterating.
        let flake = r#"{
  inputs = {
    flake-edit = {
      url = "github:a-kenji/flake-edit";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, ... }: { };
}
"#;
        let (node, child, attr, leaf) = find_input_attrset_leaf(flake, "flake-edit", "inputs");
        let mut map = HashMap::new();
        let change = Change::Remove {
            ids: vec![ChangeId::parse("unrelated").unwrap()],
        };
        let result = handle_inputs_leaf(&mut map, &node, &child, &attr, &leaf, &change);
        assert!(result.is_none(), "unrelated removal must not rewrite");
    }

    /// Locate the first `NODE_ATTRPATH_VALUE` child of an inputs block, returned
    /// as the `(SyntaxElement, SyntaxNode)` pair the per-variant handlers expect.
    fn first_attrpath_value_in_inputs(flake: &str) -> (rnix::SyntaxElement, SyntaxNode) {
        let inputs_block = parse_inputs_block(flake);
        let element = inputs_block
            .children_with_tokens()
            .find(|c| c.kind() == SyntaxKind::NODE_ATTRPATH_VALUE)
            .expect("at least one NODE_ATTRPATH_VALUE child");
        let node = element.as_node().unwrap().clone();
        (element, node)
    }

    #[test]
    fn insert_added_input_appends_flat_url_after_last_entry() {
        let flake = r#"{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, ... }: { };
}
"#;
        let inputs_block = parse_inputs_block(flake);
        let (child, child_node) = first_attrpath_value_in_inputs(flake);
        let result = insert_added_input_into_block(
            &inputs_block,
            &child,
            &child_node,
            "flake-utils",
            "github:numtide/flake-utils",
            true,
        );
        let text = result.to_string();
        assert!(text.contains("nixpkgs.url ="), "got:\n{text}");
        assert!(
            text.contains("flake-utils.url = \"github:numtide/flake-utils\""),
            "got:\n{text}"
        );
    }

    #[test]
    fn insert_added_input_appends_flake_false_pair_when_flake_disabled() {
        let flake = r#"{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, ... }: { };
}
"#;
        let inputs_block = parse_inputs_block(flake);
        let (child, child_node) = first_attrpath_value_in_inputs(flake);
        let result = insert_added_input_into_block(
            &inputs_block,
            &child,
            &child_node,
            "naked",
            "github:owner/naked",
            false,
        );
        let text = result.to_string();
        assert!(
            text.contains("naked.url = \"github:owner/naked\""),
            "got:\n{text}"
        );
        assert!(
            text.contains("naked.flake = false"),
            "flake = false attr must be emitted alongside the url, got:\n{text}"
        );
    }

    #[test]
    fn insert_added_input_uses_attrset_style_when_block_does() {
        let flake = r#"{
  inputs = {
    nixpkgs = { url = "github:NixOS/nixpkgs/nixos-unstable"; };
  };

  outputs = { self, ... }: { };
}
"#;
        let inputs_block = parse_inputs_block(flake);
        let (child, child_node) = first_attrpath_value_in_inputs(flake);
        let result = insert_added_input_into_block(
            &inputs_block,
            &child,
            &child_node,
            "flake-utils",
            "github:numtide/flake-utils",
            true,
        );
        let text = result.to_string();
        assert!(
            text.contains("flake-utils = {")
                && text.contains("url = \"github:numtide/flake-utils\""),
            "added entry must be in attrset shape to match siblings, got:\n{text}"
        );
        assert!(
            !text.contains("flake-utils.url ="),
            "attrset block must not fall back to flat shape, got:\n{text}"
        );
    }

    /// Find the first `NODE_ATTRPATH` whose last ident is `follows` anywhere
    /// in the parsed flake. Mirrors how `handle_input_attrpath` reaches the
    /// follows attr in production.
    fn find_follows_attrpath(flake: &str) -> SyntaxNode {
        fn search(node: &SyntaxNode) -> Option<SyntaxNode> {
            for child in node.children() {
                if child.kind() == SyntaxKind::NODE_ATTRPATH
                    && child
                        .children()
                        .last()
                        .map(|c| c.to_string() == "follows")
                        .unwrap_or(false)
                {
                    return Some(child);
                }
                if let Some(found) = search(&child) {
                    return Some(found);
                }
            }
            None
        }
        search(&Root::parse(flake).syntax()).expect("no follows attrpath in flake")
    }

    /// Extract the chain (attrpath children minus `inputs` / `follows`) the same
    /// way [`super::handle_follows_attr`] does.
    fn chain_for(attrpath: &SyntaxNode) -> Vec<SyntaxNode> {
        attrpath
            .children()
            .filter(|c| c.to_string() != "inputs" && c.to_string() != "follows")
            .collect()
    }

    fn segs_to_strings(segs: &[Segment]) -> Vec<String> {
        segs.iter().map(|s| s.as_str().to_string()).collect()
    }

    #[test]
    fn resolve_follows_owner_uses_chain_first_when_ctx_is_none() {
        let flake = r#"{
  inputs.flake-edit.inputs.nixpkgs.follows = "nixpkgs";
}
"#;
        let attrpath = find_follows_attrpath(flake);
        let chain = chain_for(&attrpath);
        let (owner, nested) = resolve_follows_owner_and_nested(&attrpath, &chain, &None);
        assert_eq!(owner.as_str(), "flake-edit");
        assert_eq!(segs_to_strings(&nested), vec!["nixpkgs".to_string()]);
    }

    #[test]
    fn resolve_follows_owner_uses_ctx_when_attrpath_starts_with_inputs() {
        let flake = r#"{
  inputs.flake-edit = {
    inputs.nixpkgs.follows = "nixpkgs";
  };
}
"#;
        let attrpath = find_follows_attrpath(flake);
        let chain = chain_for(&attrpath);
        let ctx: Option<Context> = Some(Segment::from_unquoted("flake-edit").unwrap().into());
        let (owner, nested) = resolve_follows_owner_and_nested(&attrpath, &chain, &ctx);
        assert_eq!(owner.as_str(), "flake-edit");
        assert_eq!(segs_to_strings(&nested), vec!["nixpkgs".to_string()]);
    }

    #[test]
    fn resolve_follows_owner_strips_repeated_owner_when_chain_first_matches_ctx() {
        let flake = r#"{
  flake-edit.nixpkgs.follows = "nixpkgs";
}
"#;
        let attrpath = find_follows_attrpath(flake);
        let chain = chain_for(&attrpath);
        let ctx: Option<Context> = Some(Segment::from_unquoted("flake-edit").unwrap().into());
        let (owner, nested) = resolve_follows_owner_and_nested(&attrpath, &chain, &ctx);
        assert_eq!(owner.as_str(), "flake-edit");
        assert_eq!(segs_to_strings(&nested), vec!["nixpkgs".to_string()]);
    }

    #[test]
    fn resolve_follows_owner_keeps_full_chain_when_ctx_owner_is_unrelated() {
        let flake = r#"{
  bar.nixpkgs.follows = "nixpkgs";
}
"#;
        let attrpath = find_follows_attrpath(flake);
        let chain = chain_for(&attrpath);
        let ctx: Option<Context> = Some(Segment::from_unquoted("foo").unwrap().into());
        let (owner, nested) = resolve_follows_owner_and_nested(&attrpath, &chain, &ctx);
        assert_eq!(owner.as_str(), "foo");
        assert_eq!(
            segs_to_strings(&nested),
            vec!["bar".to_string(), "nixpkgs".to_string()]
        );
    }

    #[test]
    fn handle_url_attr_strips_flat_input_on_matching_remove() {
        let flake = "{
  inputs = {
    nixpkgs.url = \"github:NixOS/nixpkgs/nixos-unstable\";
    flake-edit.url = \"github:a-kenji/flake-edit\";
  };

  outputs = { self, ... }: { };
}
";
        let change = Change::Remove {
            ids: vec![ChangeId::parse("flake-edit").unwrap()],
        };
        let result = apply(flake, &change);
        assert_eq!(
            result,
            "{
  inputs = {
    nixpkgs.url = \"github:NixOS/nixpkgs/nixos-unstable\";
  };

  outputs = { self, ... }: { };
}
"
        );
    }

    #[test]
    fn handle_url_attr_rewrites_uri_for_matching_flat_change() {
        let flake = "{
  inputs = {
    nixpkgs.url = \"github:NixOS/nixpkgs/nixos-unstable\";
  };

  outputs = { self, ... }: { };
}
";
        let change = Change::Change {
            id: Some(ChangeId::parse("nixpkgs").unwrap()),
            uri: Some("github:NixOS/nixpkgs/nixos-23.11".to_string()),
        };
        let result = apply(flake, &change);
        assert_eq!(
            result,
            "{
  inputs = {
    nixpkgs.url = \"github:NixOS/nixpkgs/nixos-23.11\";
  };

  outputs = { self, ... }: { };
}
"
        );
    }

    #[test]
    fn handle_url_attr_leaves_flat_input_alone_for_unrelated_change() {
        let flake = "{
  inputs = {
    nixpkgs.url = \"github:NixOS/nixpkgs/nixos-unstable\";
  };

  outputs = { self, ... }: { };
}
";
        let change = Change::Change {
            id: Some(ChangeId::parse("flake-edit").unwrap()),
            uri: Some("github:a-kenji/flake-edit".to_string()),
        };
        assert!(apply_maybe(flake, &change).is_none());
    }

    #[test]
    fn handle_attrpath_follows_strips_depth_one_on_owner_remove() {
        let flake = "{
  inputs = {
    nixpkgs.url = \"github:NixOS/nixpkgs/nixos-unstable\";
    flake-edit.follows = \"nixpkgs\";
  };

  outputs = { self, ... }: { };
}
";
        let change = Change::Remove {
            ids: vec![ChangeId::parse("flake-edit").unwrap()],
        };
        let result = apply(flake, &change);
        assert_eq!(
            result,
            "{
  inputs = {
    nixpkgs.url = \"github:NixOS/nixpkgs/nixos-unstable\";
  };

  outputs = { self, ... }: { };
}
"
        );
    }

    #[test]
    fn handle_attrpath_follows_strips_depth_two_on_remove() {
        let flake = "{
  inputs = {
    nixpkgs.url = \"github:NixOS/nixpkgs/nixos-unstable\";
    flake-edit.url = \"github:a-kenji/flake-edit\";
    flake-edit.inputs.nixpkgs.follows = \"nixpkgs\";
  };

  outputs = { self, ... }: { };
}
";
        let change = Change::Remove {
            ids: vec![ChangeId::parse("flake-edit.nixpkgs").unwrap()],
        };
        let result = apply(flake, &change);
        assert_eq!(
            result,
            "{
  inputs = {
    nixpkgs.url = \"github:NixOS/nixpkgs/nixos-unstable\";
    flake-edit.url = \"github:a-kenji/flake-edit\";
  };

  outputs = { self, ... }: { };
}
"
        );
    }

    #[test]
    fn handle_attrpath_follows_strips_depth_three_on_full_path_remove() {
        let flake = "{
  inputs = {
    nixpkgs.url = \"github:NixOS/nixpkgs/nixos-unstable\";
    flake-edit.url = \"github:a-kenji/flake-edit\";
    flake-edit.inputs.helper.inputs.nixpkgs.follows = \"nixpkgs\";
  };

  outputs = { self, ... }: { };
}
";
        let change = Change::Remove {
            ids: vec![ChangeId::parse("flake-edit.helper.nixpkgs").unwrap()],
        };
        let result = apply(flake, &change);
        assert_eq!(
            result,
            "{
  inputs = {
    nixpkgs.url = \"github:NixOS/nixpkgs/nixos-unstable\";
    flake-edit.url = \"github:a-kenji/flake-edit\";
  };

  outputs = { self, ... }: { };
}
"
        );
    }

    #[test]
    fn handle_attrpath_follows_leaves_depth_three_alone_for_unrelated_remove() {
        let flake = "{
  inputs = {
    nixpkgs.url = \"github:NixOS/nixpkgs/nixos-unstable\";
    flake-edit.url = \"github:a-kenji/flake-edit\";
    flake-edit.inputs.helper.inputs.nixpkgs.follows = \"nixpkgs\";
  };

  outputs = { self, ... }: { };
}
";
        let change = Change::Remove {
            ids: vec![ChangeId::parse("flake-edit.helper.flake-utils").unwrap()],
        };
        assert!(apply_maybe(flake, &change).is_none());
    }

    #[test]
    fn handle_attrpath_follows_records_indirect_edge_for_change_none() {
        let flake = "{
  inputs = {
    nixpkgs.url = \"github:NixOS/nixpkgs/nixos-unstable\";
    flake-edit.url = \"github:a-kenji/flake-edit\";
    flake-edit.inputs.nixpkgs.follows = \"nixpkgs\";
  };

  outputs = { self, ... }: { };
}
";
        let mut walker = Walker::new(flake);
        let _ = walker.walk(&Change::None).expect("walker error");
        let entry = walker
            .inputs
            .get("flake-edit")
            .expect("owner must be present in walker map");
        let expected = vec![crate::input::Follows::Indirect {
            path: AttrPath::parse("nixpkgs").unwrap(),
            target: Some(AttrPath::parse("nixpkgs").unwrap()),
        }];
        assert_eq!(entry.follows, expected);
    }

    #[test]
    fn handle_url_attr_records_flat_input_url_for_change_none() {
        let flake = "{
  inputs = {
    nixpkgs.url = \"github:NixOS/nixpkgs/nixos-unstable\";
    flake-edit.url = \"github:a-kenji/flake-edit\";
  };

  outputs = { self, ... }: { };
}
";
        let mut walker = Walker::new(flake);
        let _ = walker.walk(&Change::None).expect("walker error");
        assert_eq!(
            walker.inputs["nixpkgs"].url(),
            "github:NixOS/nixpkgs/nixos-unstable"
        );
        assert_eq!(
            walker.inputs["flake-edit"].url(),
            "github:a-kenji/flake-edit"
        );
    }
}
