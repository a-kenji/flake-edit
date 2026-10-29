use rnix::{SyntaxKind, SyntaxNode};

use crate::change::Change;
use crate::follows::path::{follows_idents_bare, follows_idents_prefixed};
use crate::follows::{AttrPath, Segment, strip_outer_quotes};

use super::context::Context;
use super::flake_attr_set;
use super::node::{
    FollowsKind, adjacent_whitespace_index, empty_node, extract_indent, get_sibling_whitespace,
    insertion_index_after, is_attrset_content_empty, last_line_with_newline, make_attrset_url_attr,
    make_attrset_url_flake_false_attr, make_flake_false_attr, make_quoted_string, make_url_attr,
    parse_node, remove_child_with_whitespace, should_remove_input, substitute_child,
    trailing_inline_comments, uses_attrset_style,
};

/// Walk the `inputs` section of a `flake.nix`, applying `change`.
///
/// The input model is recorded by [`super::record`]. This traversal only
/// rewrites.
pub(crate) fn walk_inputs(
    node: SyntaxNode,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    if node.kind() == SyntaxKind::NODE_ATTRPATH
        && let Some(result) = handle_attrpath_follows(&node, change)
    {
        return Some(result);
    }

    match change {
        Change::Add { .. } => apply_add(node, ctx, change),
        Change::Follows { .. } => apply_follows(node, ctx, change),
        // `Remove`/`Change` rewrite a single matched child in place and only
        // need to traverse children, never rebuilding the block.
        // `Change::None` and the toggle changes never reach this walk:
        // `Walker::walk` returns after recording for `None`, and `FlakeEdit`
        // edits toggles through `walk::toggle` directly. Grouped here for
        // exhaustiveness.
        Change::None
        | Change::Remove { .. }
        | Change::Change { .. }
        | Change::Toggle { .. }
        | Change::ToggleRemove { .. } => walk_children(&node, ctx, change),
    }
}

fn walk_children(node: &SyntaxNode, ctx: &Option<Context>, change: &Change) -> Option<SyntaxNode> {
    for child in node.children_with_tokens() {
        match child.kind() {
            SyntaxKind::NODE_ATTRPATH_VALUE => {
                if let Some(result) = handle_child_attrpath_value(node, &child, ctx, change) {
                    return Some(result);
                }
            }
            SyntaxKind::NODE_IDENT => {
                if let Some(result) = handle_child_ident(&child, change) {
                    return Some(result);
                }
            }
            _ => {}
        }
    }
    None
}

/// An empty `inputs = { }` block has no `NODE_ATTRPATH_VALUE` children for
/// [`walk_children`] to splice into, so the rebuild step below kicks in
/// only when traversal returned nothing.
fn apply_add(node: SyntaxNode, ctx: &Option<Context>, change: &Change) -> Option<SyntaxNode> {
    if let Some(result) = walk_children(&node, ctx, change) {
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
    let id = id.input();

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
fn insert_into_empty_inputs(node: &SyntaxNode, id: &Segment, uri: &str, flake: bool) -> SyntaxNode {
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
        node.green().to_owned()
    };

    let brace_index = green
        .children()
        .position(|c| c.as_token().map(|t| t.text() == "}").unwrap_or(false))
        .unwrap_or(green.children().count());

    green = green.insert_child(brace_index, uri_node.green().to_owned().into());
    green = green.insert_child(
        brace_index,
        parse_node(&entry_indent).green().to_owned().into(),
    );

    let mut offset = 2;
    if !flake {
        let no_flake = make_flake_false_attr(id);
        green = green.insert_child(
            brace_index + offset,
            parse_node(&entry_indent).green().to_owned().into(),
        );
        offset += 1;
        green = green.insert_child(brace_index + offset, no_flake.green().to_owned().into());
        offset += 1;
    }

    green = green.insert_child(
        brace_index + offset,
        parse_node(&closing_indent).green().to_owned().into(),
    );

    SyntaxNode::new_root(green)
}

/// When the parent is declared in attrset shape (`parent = { ... }`)
/// [`handle_input_attr_set`] owns the follows insertion via the child
/// traversal step. This function only handles the flat-toplevel shape
/// (`parent.url = "..."`), where the follows entry must be spliced next
/// to the `url` declaration before traversal happens.
fn apply_follows(node: SyntaxNode, ctx: &Option<Context>, change: &Change) -> Option<SyntaxNode> {
    let Change::Follows { input, target } = change else {
        return None;
    };

    if ctx.is_none()
        && node.kind() == SyntaxKind::NODE_ATTR_SET
        && let Some(result) = insert_flat_toplevel_follows(&node, input, target)
    {
        return result;
    }

    walk_children(&node, ctx, change)
}

/// The `Option<Option<_>>` return distinguishes "this branch claims the
/// rewrite" from "this branch does not apply". `Some(Some(node))` means
/// the rewrite has been performed (or the desired follows already exists,
/// in which case `node` is the unchanged input). `Some(None)` is reserved
/// for hits that intentionally short-circuit without producing a rewrite.
/// `None` lets the caller fall through to child traversal.
fn insert_flat_toplevel_follows(
    node: &SyntaxNode,
    input: &crate::change::ChangeId,
    target: &AttrPath,
) -> Option<Option<SyntaxNode>> {
    let full_path = input.path();
    let parent_id = input.input();
    let parent_id_str = parent_id.as_str();
    let target_str = target.to_flake_follows_string();

    if full_path.len() >= 2 && !flat_input_has_nested_block(node, parent_id_str) {
        if let Some(result) = find_existing_flat_follows(node, full_path.segments(), &target_str) {
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
        .insert_child(insert_index, follows_node.green().to_owned().into());

    if let Some(whitespace) = get_sibling_whitespace(ref_child) {
        let ws_str = whitespace.to_string();
        let ws_node = parse_node(last_line_with_newline(&ws_str));
        green = green.insert_child(insert_index, ws_node.green().to_owned().into());
    }

    Some(SyntaxNode::new_root(green))
}

/// Handle a flat-style URL attribute (`inputs.foo.url = "..."`), returning the
/// replacement node when `change` modifies it.
fn handle_flat_url(input_id: &SyntaxNode, change: &Change) -> Option<SyntaxNode> {
    let id_seg = Segment::from_syntax_or_sentinel(input_id);
    let id_str = id_seg.as_str().to_string();

    if should_remove_input(change, &id_seg) {
        return Some(empty_node());
    }

    if let Change::Change {
        id: Some(change_id),
        uri: Some(new_uri),
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
fn handle_flat_flake(input_id: &SyntaxNode, change: &Change) -> Option<SyntaxNode> {
    let id_seg = Segment::from_syntax_or_sentinel(input_id);

    if should_remove_input(change, &id_seg) {
        return Some(empty_node());
    }

    None
}

/// Handle a nested input declaration (`inputs.foo = { url = "..."; ... }`),
/// returning the replacement node when `change` modifies it.
fn handle_nested_input(
    input_id: &SyntaxNode,
    nested_attr: &SyntaxNode,
    change: &Change,
) -> Option<SyntaxNode> {
    let id_seg = Segment::from_syntax_or_sentinel(input_id);

    for attr in nested_attr.children() {
        if attr.children().next().is_some() && should_remove_input(change, &id_seg) {
            return Some(empty_node());
        }
        if let Change::Change {
            id: Some(change_id),
            uri: Some(new_uri),
        } = change
            && *change_id.input() == id_seg
            && change_id.follows().is_none()
            && let Some(key) = attr.first_child()
            && key.to_string() == "url"
            && let Some(url) = key.next_sibling()
        {
            let new_attr = substitute_child(&attr, url.index(), &make_quoted_string(new_uri));
            return Some(substitute_child(nested_attr, attr.index(), &new_attr));
        }

        let context: Context = id_seg.clone().into();
        if walk_input(&attr, &Some(context), change).is_some() {
            let replacement = remove_child_with_whitespace(nested_attr, &attr, attr.index());
            return Some(replacement);
        }
    }

    None
}

/// Handle a `NODE_IDENT` child during input walking, covering flat-style
/// declarations like `inputs.nixpkgs.url = "..."`.
fn handle_child_ident(child: &rnix::SyntaxElement, change: &Change) -> Option<SyntaxNode> {
    let child_node = child.as_node()?;
    if child.to_string() != "inputs" || child_node.prev_sibling().is_some() {
        return None;
    }
    let parent_sibling = child_node.parent().and_then(|p| p.next_sibling());

    // `inputs` ident with a sibling (e.g. `inputs.foo.url = ...` or
    // `inputs.foo = { ... }`).
    if let Some(next_sibling) = child_node.next_sibling() {
        // `NODE_STRING` covers quoted key segments like `inputs."master".url`
        // and `inputs."nixos-24.11".url`, where a dot inside the quotes belongs
        // to the name rather than separating a path.
        // `Segment::from_syntax_or_sentinel` unquotes it downstream.
        match next_sibling.kind() {
            SyntaxKind::NODE_IDENT | SyntaxKind::NODE_STRING => {
                if let Some(url_id) = next_sibling.next_sibling() {
                    if url_id.kind() == SyntaxKind::NODE_IDENT && parent_sibling.is_some() {
                        if url_id.to_string() == "url" {
                            if let Some(result) = handle_flat_url(&next_sibling, change) {
                                return Some(result);
                            }
                        } else if url_id.to_string() == "flake"
                            && let Some(result) = handle_flat_flake(&next_sibling, change)
                        {
                            return Some(result);
                        }
                    }
                } else if let Some(nested_attr) = &parent_sibling
                    && let Some(result) = handle_nested_input(&next_sibling, nested_attr, change)
                {
                    return Some(result);
                }
            }
            SyntaxKind::NODE_ATTR_SET => {}
            _ => {}
        }
    }

    let id = change.is_remove().then(|| change.id()).flatten()?;
    let owner = child_node.next_sibling()?;
    let rest: Vec<SyntaxNode> =
        std::iter::successors(owner.next_sibling(), |n| n.next_sibling()).collect();
    remove_targeted_binding(
        &rest,
        parent_sibling.as_ref()?,
        &id,
        &Segment::from_syntax_or_sentinel(&owner),
        &[],
    )
}

/// Declaration to put in place of `entry` when a removal deletes it.
///
/// `entry` is an input-declaring binding, either inside the `inputs = { }`
/// block or a flat `inputs.<owner>...` binding of the flake attrset.
pub(crate) fn redeclare_removed_owner(entry: &SyntaxNode, change: &Change) -> Option<SyntaxNode> {
    if !change.is_remove() {
        return None;
    }
    let attrpath = entry.first_child()?;
    let idents: Vec<SyntaxNode> = attrpath.children().collect();
    let flat_toplevel = idents.len() >= 2 && idents[0].to_string() == "inputs";
    let owner_ident = if flat_toplevel {
        &idents[1]
    } else {
        idents.first()?
    };
    let owner = Segment::from_syntax_or_sentinel(owner_ident);
    if change
        .id()
        .is_some_and(|id| id.path().len() == 1 && *id.input() == owner)
    {
        return None;
    }
    if declares_input_elsewhere(entry, &owner) {
        return None;
    }
    let path = if flat_toplevel {
        format!("inputs.{owner_ident}")
    } else {
        owner_ident.to_string()
    };
    parse_node(&format!("{{ {path}.url = \"flake:{}\"; }}", owner.as_str()))
        .descendants()
        .find(|n| n.kind() == SyntaxKind::NODE_ATTRPATH_VALUE)
}

/// Whether any input-declaring binding other than `entry` names `owner`.
fn declares_input_elsewhere(entry: &SyntaxNode, owner: &Segment) -> bool {
    let Some(flake) = entry
        .ancestors()
        .last()
        .and_then(|root| flake_attr_set(&root))
    else {
        return false;
    };
    let names_owner = |binding: &SyntaxNode, ident: Option<SyntaxNode>| {
        binding != entry && ident.is_some_and(|i| Segment::from_syntax_or_sentinel(&i) == *owner)
    };
    flake
        .children()
        .filter(|c| c.kind() == SyntaxKind::NODE_ATTRPATH_VALUE)
        .any(|toplevel| {
            let Some(attrpath) = toplevel.first_child() else {
                return false;
            };
            let mut idents = attrpath.children();
            if idents.next().is_none_or(|i| i.to_string() != "inputs") {
                return false;
            }
            if let Some(ident) = idents.next() {
                return names_owner(&toplevel, Some(ident));
            }
            attrpath.next_sibling().is_some_and(|block| {
                block
                    .children()
                    .filter(|c| c.kind() == SyntaxKind::NODE_ATTRPATH_VALUE)
                    .any(|binding| {
                        let ident = binding.first_child().and_then(|p| p.first_child());
                        names_owner(&binding, ident)
                    })
            })
        })
}

/// Handle a `NODE_ATTRPATH_VALUE` child during input walking.
fn handle_child_attrpath_value(
    parent: &SyntaxNode,
    child: &rnix::SyntaxElement,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    let child_node = child.as_node().unwrap();
    let in_inputs_block = ctx.is_none();

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

    let walked = walk_input(child_node, &ctx, change).or_else(|| {
        in_inputs_block
            .then(|| remove_targeted_block_entry(child_node, change))
            .flatten()
    });
    if let Some(mut replacement) = walked {
        if in_inputs_block
            && replacement.text().is_empty()
            && let Some(redeclared) = redeclare_removed_owner(child_node, change)
        {
            replacement = redeclared;
        }
        let mut green = parent
            .green()
            .replace_child(child.index(), replacement.green().to_owned().into());

        if replacement.text().is_empty() {
            let mut to_remove: Vec<usize> = trailing_inline_comments(child)
                .iter()
                .map(|t| t.index())
                .collect();
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
            id.input(),
            uri,
            *flake,
        ));
    }

    None
}

/// [`remove_targeted_binding`] for a whole block entry.
fn remove_targeted_block_entry(entry: &SyntaxNode, change: &Change) -> Option<SyntaxNode> {
    let id = change.is_remove().then(|| change.id()).flatten()?;
    let path = entry.first_child()?;
    let value = path.next_sibling()?;
    let idents: Vec<SyntaxNode> = path.children().collect();
    let (owner, rest) = idents.split_first()?;
    let owner = Segment::from_syntax_or_sentinel(owner);
    let new_value = remove_targeted_binding(rest, &value, &id, &owner, &[])?;
    if new_value.text().is_empty() {
        return Some(new_value);
    }
    Some(substitute_child(entry, value.index(), &new_value))
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
    id: &Segment,
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
            .insert_child(insert_index, ws_node.green().to_owned().into());
        let mut offset = 1;

        if use_attrset {
            let indent = extract_indent(&ws_str);
            let uri_node = if flake {
                make_attrset_url_attr(id, uri, indent)
            } else {
                make_attrset_url_flake_false_attr(id, uri, indent)
            };
            green = green.insert_child(insert_index + offset, uri_node.green().to_owned().into());
        } else {
            let uri_node = make_url_attr(id, uri);
            green = green.insert_child(insert_index + offset, uri_node.green().to_owned().into());
            offset += 1;

            if !flake {
                let no_flake = make_flake_false_attr(id);
                let compact_ws_node = parse_node(last_line_with_newline(&ws_str));
                green = green.insert_child(
                    insert_index + offset,
                    compact_ws_node.green().to_owned().into(),
                );
                offset += 1;
                green =
                    green.insert_child(insert_index + offset, no_flake.green().to_owned().into());
            }
        }
        return SyntaxNode::new_root(green);
    }

    let uri_node = make_url_attr(id, uri);
    let mut green = parent
        .green()
        .insert_child(insert_index, uri_node.green().to_owned().into());

    if !flake {
        let no_flake = make_flake_false_attr(id);
        green = green.insert_child(insert_index + 1, no_flake.green().to_owned().into());
    }
    SyntaxNode::new_root(green)
}

/// Handle a `NODE_ATTRPATH` whose last segment is `follows`, at any depth.
///
/// The owning input is the first non-`inputs` segment of the attrpath. The
/// remaining non-`inputs` segments (excluding the trailing `follows`) form
/// the nested-input path of the declaration.
fn handle_attrpath_follows(node: &SyntaxNode, change: &Change) -> Option<SyntaxNode> {
    let parts = parse_follows_attrpath_parts(node)?;
    if parts.rest.is_empty() {
        remove_depth_one_follows_attr(&parts.owner_seg, change)
    } else {
        remove_depth_n_follows_attr(&parts, change)
    }
}

struct FollowsAttrPathParts {
    owner_seg: Segment,
    rest: Vec<Segment>,
}

/// Returns `None` for any non-follows attrpath, for an attrpath that is
/// nothing but `inputs` keywords, or when the binding has no value sibling.
fn parse_follows_attrpath_parts(node: &SyntaxNode) -> Option<FollowsAttrPathParts> {
    let children: Vec<SyntaxNode> = node.children().collect();
    let last = children.last()?;
    if last.to_string() != "follows" {
        return None;
    }

    let path_segments: Vec<Segment> = children[..children.len() - 1]
        .iter()
        .filter(|c| c.to_string() != "inputs")
        .map(Segment::from_syntax_or_sentinel)
        .collect();

    let owner_seg = path_segments.first().cloned()?;
    let rest: Vec<Segment> = path_segments[1..].to_vec();
    node.next_sibling()?;

    Some(FollowsAttrPathParts { owner_seg, rest })
}

/// `inputs.<owner>.follows = "T"`.
fn remove_depth_one_follows_attr(owner_seg: &Segment, change: &Change) -> Option<SyntaxNode> {
    if change.is_remove()
        && let Some(id) = change.id()
        && remove_targets_follows_decl(&id, owner_seg, &[])
    {
        return Some(empty_node());
    }
    None
}

/// `inputs.<owner>.inputs.<n1>…follows = "T"`.
fn remove_depth_n_follows_attr(
    parts: &FollowsAttrPathParts,
    change: &Change,
) -> Option<SyntaxNode> {
    if change.is_remove()
        && let Some(id) = change.id()
        && remove_targets_follows_decl(&id, &parts.owner_seg, &parts.rest)
    {
        return Some(empty_node());
    }
    None
}

/// `inputs.<id>.url = "..."` at the inputs-block level. With `ctx` set, the
/// same shape inside another input's attrset is a transitive URL override,
/// not a follows.
fn handle_url_attr(
    node: &SyntaxNode,
    child: &SyntaxNode,
    attr: &SyntaxNode,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    let prev_id = attr.prev_sibling()?;
    if ctx.is_some() {
        let chain: Vec<SyntaxNode> = child
            .children()
            .filter(|c| c != attr && c.to_string() != "inputs")
            .collect();
        let (owner_seg, nested_segs) = resolve_follows_owner_and_nested(child, &chain, ctx);
        if change.is_remove()
            && let Some(id) = change.id()
            && remove_targets_follows_decl(&id, &owner_seg, &nested_segs)
        {
            return Some(empty_node());
        }
        return None;
    }
    let prev_seg = Segment::from_syntax_or_sentinel(&prev_id);
    let prev_str = prev_seg.as_str().to_string();
    if change.is_remove()
        && let Some(id) = change.id()
        && id.input().as_str() == prev_str
        && id.follows().is_none()
    {
        return Some(empty_node());
    }
    if let Change::Change { id, uri } = change
        && let Some(id) = id
        && id.input().as_str() == prev_str
        && id.follows().is_none()
        && let Some(uri) = uri
        && let Some(url_node) = child.next_sibling()
    {
        let new_url = make_quoted_string(uri);
        return Some(substitute_child(node, url_node.index(), &new_url));
    }
    None
}

/// Handle a `flake = ...` binding inside an input's attrset.
fn handle_flake_attr(
    attr: &SyntaxNode,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    let attrpath = attr.parent().unwrap();
    if attr.prev_sibling().is_some() && attrpath.next_sibling().is_some() {
        // `inputs.mid.flake` and the depth-2 follows `owner.mid.leaf` share
        // their first two segments. Only the full path tells them apart.
        let chain: Vec<SyntaxNode> = attrpath
            .children()
            .filter(|c| c != attr && c.to_string() != "inputs")
            .collect();
        if change.is_remove()
            && let Some(id) = change.id()
            && !chain.is_empty()
        {
            let (owner_seg, nested_segs) = resolve_follows_owner_and_nested(&attrpath, &chain, ctx);
            if remove_targets_follows_decl(&id, &owner_seg, &nested_segs) {
                return Some(empty_node());
            }
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
    attr: &SyntaxNode,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    let attrpath = attr.parent().unwrap();

    let chain: Vec<SyntaxNode> = attrpath
        .children()
        .filter(|c| c.to_string() != "inputs" && c.to_string() != "follows")
        .collect();

    if chain.is_empty() {
        return None;
    }

    let (owner_seg, nested_segs) = resolve_follows_owner_and_nested(&attrpath, &chain, ctx);

    if change.is_remove()
        && let Some(id) = change.id()
        && remove_targets_follows_decl(&id, &owner_seg, &nested_segs)
    {
        return Some(empty_node());
    }

    None
}

/// Whether a `Remove` actually targets the follows declaration
/// `owner.nested… .follows`, as opposed to merely sharing its first two
/// segments.
///
/// [`crate::change::ChangeId`] exposes only `input` (first segment) and
/// `follows` (second), so the ctx-driven predicates answer the same thing for
/// `owner.mid` and `owner.mid.leaf`. Removing the depth-2 follows
/// `owner.mid.leaf` therefore also matched the depth-1 declaration
/// `owner.inputs.mid.follows` and deleted it.
///
/// A single-segment id names the whole owner input, where stripping its
/// nested follows lines is correct; anything longer has to match the
/// declaration's full path exactly.
fn remove_targets_follows_decl(
    id: &crate::change::ChangeId,
    owner: &Segment,
    nested: &[Segment],
) -> bool {
    let segs = id.path().segments();
    if segs.len() == 1 {
        return segs[0] == *owner;
    }
    segs.len() == nested.len() + 1
        && segs[0] == *owner
        && segs[1..].iter().zip(nested.iter()).all(|(a, b)| a == b)
}

/// Resolve the `(owner, nested)` pair for a follows attrpath.
///
/// Discriminator: the raw attrpath's first ident is `inputs` for the
/// inside-an-input-block shape.
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
    node: &SyntaxNode,
    child: &SyntaxNode,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    for attr in child.children() {
        let attr_name = attr.to_string();
        match attr_name.as_str() {
            "url" => {
                if let Some(result) = handle_url_attr(node, child, &attr, ctx, change) {
                    return Some(result);
                }
            }
            "flake" => {
                if let Some(result) = handle_flake_attr(&attr, ctx, change) {
                    return Some(result);
                }
            }
            "follows" => {
                if let Some(result) = handle_follows_attr(&attr, ctx, change) {
                    return Some(result);
                }
            }
            _ => {}
        }
    }
    None
}

/// Find the follows declaration whose full attrpath is `expected` inside
/// `container`, however the path is split between dotted keys and nested
/// attrsets: `t.follows`, `t = { follows = ...; }` and, for a deeper path,
/// `n.inputs = { t.follows = ...; }` all match. Idents are compared with
/// their outer `"..."` stripped.
///
/// `Some(None)` when the declaration already follows `target`, `Some(Some)`
/// with `container` rebuilt around the retargeted value otherwise, and
/// `None` when `container` does not declare it.
pub(super) fn retarget_existing_follows(
    container: &SyntaxNode,
    expected: &[&str],
    target: &str,
) -> Option<Option<SyntaxNode>> {
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
        if idents.len() > expected.len()
            || !idents
                .iter()
                .zip(expected.iter())
                .all(|(have, want)| strip_outer_quotes(have) == *want)
        {
            continue;
        }
        let Some(value) = attrpath.next_sibling() else {
            continue;
        };

        let new_value = if idents.len() == expected.len() {
            if strip_outer_quotes(&value.to_string()) == target {
                return Some(None);
            }
            make_quoted_string(target)
        } else if value.kind() == SyntaxKind::NODE_ATTR_SET
            && let Some(found) =
                retarget_existing_follows(&value, &expected[idents.len()..], target)
        {
            let Some(new_inner) = found else {
                return Some(None);
            };
            new_inner
        } else {
            continue;
        };
        let new_attr = substitute_child(&attr, value.index(), &new_value);
        return Some(Some(substitute_child(container, attr.index(), &new_attr)));
    }
    None
}

/// Locate an `inputs.<R0>...inputs.<RN>.follows` declaration inside an
/// input's `{ ... }` block and rebuild the surrounding `node`.
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
    let rebuilt = match retarget_existing_follows(attr_set, &expected, target)? {
        None => node.clone(),
        Some(new_attr_set) => substitute_child(node, attr_set.index(), &new_attr_set),
    };
    Some(Some(rebuilt))
}

/// Locate a `<S0>.inputs.<S1>...inputs.<SN>.follows` declaration at
/// `node`'s level (e.g. inside an `inputs = { ... }` block) and rebuild it
/// the same way as [`find_existing_nested_follows`].
fn find_existing_flat_follows(
    node: &SyntaxNode,
    path: &[Segment],
    target: &str,
) -> Option<Option<SyntaxNode>> {
    let expected = follows_idents_bare(path);
    let rebuilt = retarget_existing_follows(node, &expected, target)?;
    Some(Some(rebuilt.unwrap_or_else(|| node.clone())))
}

fn handle_url_leaf(
    node: &SyntaxNode,
    child: &SyntaxNode,
    attr: &SyntaxNode,
    leaf: &SyntaxNode,
    change: &Change,
) -> Option<SyntaxNode> {
    let id_node = child.prev_sibling().unwrap();
    let id_seg = Segment::from_syntax_or_sentinel(&id_node);
    let id_str = id_seg.as_str().to_string();

    if change.is_remove()
        && let Some(id) = change.id()
        && id.input().as_str() == id_str
        && id.follows().is_none()
    {
        return Some(empty_node());
    }

    if let Change::Change {
        id: Some(change_id),
        uri: Some(new_uri),
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

/// Nested input path an entry's attrpath adds to the attrset it lives in:
/// `mid.inputs.leaf.follows` yields `[mid, leaf]`, `mid.flake` and
/// `mid = { ... }` yield `[mid]`, `follows` and `inputs = { ... }` yield `[]`.
fn nested_entry_segments(idents: &[SyntaxNode], value_is_attrset: bool) -> Vec<Segment> {
    let idents = match idents.split_last() {
        Some((last, init))
            if !value_is_attrset
                && matches!(last.to_string().as_str(), "follows" | "flake" | "url") =>
        {
            init
        }
        _ => idents,
    };
    idents
        .iter()
        .filter(|c| c.to_string() != "inputs")
        .map(Segment::from_syntax_or_sentinel)
        .collect()
}

/// Remove the binding of `attrset` that a `Remove` of `id` targets.
fn remove_targeted_entry(
    attrset: &SyntaxNode,
    id: &crate::change::ChangeId,
    owner: &Segment,
    prefix: &[Segment],
) -> Option<SyntaxNode> {
    for entry in attrset.children() {
        if entry.kind() != SyntaxKind::NODE_ATTRPATH_VALUE {
            continue;
        }
        let Some(path) = entry.first_child() else {
            continue;
        };
        let Some(value) = path.next_sibling() else {
            continue;
        };
        let idents: Vec<SyntaxNode> = path.children().collect();
        if let Some(new_value) = remove_targeted_binding(&idents, &value, id, owner, prefix) {
            if new_value.text().is_empty() {
                return Some(remove_child_with_whitespace(attrset, &entry, entry.index()));
            }
            let new_entry = substitute_child(&entry, value.index(), &new_value);
            return Some(substitute_child(attrset, entry.index(), &new_entry));
        }
    }
    None
}

fn remove_targeted_binding(
    idents: &[SyntaxNode],
    value: &SyntaxNode,
    id: &crate::change::ChangeId,
    owner: &Segment,
    prefix: &[Segment],
) -> Option<SyntaxNode> {
    let value_is_attrset = value.kind() == SyntaxKind::NODE_ATTR_SET;
    let mut nested = prefix.to_vec();
    nested.extend(nested_entry_segments(idents, value_is_attrset));
    if remove_targets_follows_decl(id, owner, &nested) {
        return Some(empty_node());
    }

    let id_segs = id.path().segments();
    let leads_to_id = value_is_attrset
        && id_segs.len() > nested.len() + 1
        && id_segs[0] == *owner
        && id_segs[1..].starts_with(&nested);
    if !leads_to_id {
        return None;
    }
    let new_value = remove_targeted_entry(value, id, owner, &nested)?;
    if is_attrset_content_empty(&new_value) {
        return Some(empty_node());
    }
    Some(new_value)
}

fn handle_inputs_leaf(
    node: &SyntaxNode,
    child: &SyntaxNode,
    change: &Change,
) -> Option<SyntaxNode> {
    let id_node = child.prev_sibling().unwrap();
    let context: Context = Segment::from_syntax_or_sentinel(&id_node).into();
    let replacement = walk_inputs(child.clone(), &Some(context), change)?;
    Some(substitute_child(node, child.index(), &replacement))
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
            .insert_child(insert_index, follows_node.green().to_owned().into());
        if let Some(whitespace) = get_sibling_whitespace(&last_attr) {
            let ws_str = whitespace.to_string();
            let ws_node = parse_node(last_line_with_newline(&ws_str));
            green = green.insert_child(insert_index, ws_node.green().to_owned().into());
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
        let mut green = inputs_block.green().to_owned();
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
        green = green.insert_child(insert_index, follows_node.green().to_owned().into());
        green = green.insert_child(
            insert_index,
            parse_node(&entry_indent).green().to_owned().into(),
        );
        return SyntaxNode::new_root(green);
    }

    let ws_index = inputs_block
        .children_with_tokens()
        .find(|t| t.kind() == SyntaxKind::TOKEN_WHITESPACE)
        .map(|t| t.index());
    let mut green = if let Some(idx) = ws_index {
        inputs_block.green().remove_child(idx)
    } else {
        inputs_block.green().to_owned()
    };

    let brace_index = green
        .children()
        .position(|c| c.as_token().map(|t| t.text() == "}").unwrap_or(false))
        .unwrap_or(green.children().count());

    green = green.insert_child(
        brace_index,
        parse_node(&closing_indent).green().to_owned().into(),
    );
    green = green.insert_child(brace_index, follows_node.green().to_owned().into());
    green = green.insert_child(
        brace_index,
        parse_node(&entry_indent).green().to_owned().into(),
    );

    SyntaxNode::new_root(green)
}

fn handle_input_attr_set(
    node: &SyntaxNode,
    child: &SyntaxNode,
    change: &Change,
) -> Option<SyntaxNode> {
    for attr in child.children() {
        for leaf in attr.children() {
            let leaf_text = leaf.to_string();
            if leaf_text == "url"
                && let Some(result) = handle_url_leaf(node, child, &attr, &leaf, change)
            {
                return Some(result);
            }

            if leaf_text.starts_with("inputs")
                && let Some(result) = handle_inputs_leaf(node, child, change)
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
                // An existing declaration anywhere in the block, in any
                // split of its path, is retargeted in place. Inserting a
                // second one would define the attribute twice.
                if let Some(result) = find_existing_nested_follows(node, child, &rest, &target_str)
                {
                    return result;
                }

                if let Some(result) =
                    merge_follow_into_inputs_block(node, child, &rest, &target_str)
                {
                    return Some(result);
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
                        .insert_child(insert_index, follows_node.green().to_owned().into());

                    if let Some(whitespace) = get_sibling_whitespace(last_child) {
                        green =
                            green.insert_child(insert_index, whitespace.green().to_owned().into());
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
                            .insert_child(insert_index, follows_node.green().to_owned().into());

                        if let Some(whitespace) = get_sibling_whitespace(last_child) {
                            green = green
                                .insert_child(insert_index, whitespace.green().to_owned().into());
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
    node: &SyntaxNode,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    for child in node.children() {
        if child.kind() == SyntaxKind::NODE_ATTRPATH
            && let Some(result) = handle_input_attrpath(node, &child, ctx, change)
        {
            return Some(result);
        }

        if child.kind() == SyntaxKind::NODE_ATTR_SET
            && let Some(result) = handle_input_attr_set(node, &child, change)
        {
            return Some(result);
        }
    }
    None
}

#[cfg(test)]
mod tests {

    use rnix::{Root, SyntaxKind, SyntaxNode};

    use super::{
        apply_add, apply_follows, handle_inputs_leaf, handle_url_leaf,
        insert_added_input_into_block, resolve_follows_owner_and_nested, walk_children,
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

    /// Removing a depth-2 follows must not take out a depth-1 follows that
    /// happens to name the same middle input.
    ///
    /// `ChangeId` only models two segments (`input`, `follows`), so the stale
    /// path `atuin.flake-parts.nixpkgs-lib` and the load-bearing declaration
    /// `atuin.inputs.flake-parts.follows` collapse to the same
    /// `(atuin, flake-parts)` pair. The existing depth-2 tests dodge this by
    /// giving the middle input no follows of its own.
    #[test]
    fn remove_depth_two_follows_keeps_depth_one_follows_of_same_name() {
        let flake = r#"{
  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    nixpkgs-lib.url = "github:nix-community/nixpkgs.lib";
    atuin.url = "github:atuinsh/atuin";
    atuin.inputs.flake-parts.follows = "flake-parts";
    atuin.inputs.flake-parts.inputs.nixpkgs-lib.follows = "nixpkgs-lib";
  };

  outputs = { self, ... }: { };
}
"#;
        let change = Change::Remove {
            ids: vec![ChangeId::parse("atuin.flake-parts.nixpkgs-lib").unwrap()],
        };
        let result = apply(flake, &change);

        assert!(
            !result.contains("inputs.nixpkgs-lib.follows"),
            "depth-2 follows should be removed, got:\n{result}"
        );
        assert!(
            result.contains("atuin.inputs.flake-parts.follows = \"flake-parts\""),
            "depth-1 follows naming the same middle input must remain intact, got:\n{result}"
        );
    }

    const INPUTS_ATTRSET_SHARED_PREFIX: &str = r#"{
  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    nixpkgs-lib.url = "github:nix-community/nixpkgs.lib";
    atuin = {
      url = "github:atuinsh/atuin";
      inputs = {
        flake-parts.follows = "flake-parts";
        flake-parts.inputs.nixpkgs-lib.follows = "nixpkgs-lib";
      };
    };
  };
  outputs = { self, ... }: { };
}
"#;

    const FLAKE_FALSE_SHARED_PREFIX: &str = r#"{
  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    nixpkgs-lib.url = "github:nix-community/nixpkgs.lib";
    atuin = {
      url = "github:atuinsh/atuin";
      inputs.flake-parts.flake = false;
      inputs.flake-parts.inputs.nixpkgs-lib.follows = "nixpkgs-lib";
    };
  };
  outputs = { self, ... }: { };
}
"#;

    const NESTED_ATTRSET_SHARED_PREFIX: &str = r#"{
  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    nixpkgs-lib.url = "github:nix-community/nixpkgs.lib";
    atuin = {
      url = "github:atuinsh/atuin";
      inputs = {
        flake-parts = {
          follows = "flake-parts";
          inputs.nixpkgs-lib.follows = "nixpkgs-lib";
        };
      };
    };
  };
  outputs = { self, ... }: { };
}
"#;

    fn remove(flake_text: &str, id: &str) -> String {
        let change = Change::Remove {
            ids: vec![ChangeId::parse(id).unwrap()],
        };
        apply_until_fixed(flake_text, &change)
    }

    #[test]
    fn remove_depth_two_follows_keeps_depth_one_follows_in_inputs_attrset() {
        let expected = r#"{
  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    nixpkgs-lib.url = "github:nix-community/nixpkgs.lib";
    atuin = {
      url = "github:atuinsh/atuin";
      inputs = {
        flake-parts.follows = "flake-parts";
      };
    };
  };
  outputs = { self, ... }: { };
}
"#;
        assert_eq!(
            remove(
                INPUTS_ATTRSET_SHARED_PREFIX,
                "atuin.flake-parts.nixpkgs-lib"
            ),
            expected
        );
    }

    #[test]
    fn remove_depth_two_follows_keeps_flake_false_of_same_name() {
        let expected = r#"{
  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    nixpkgs-lib.url = "github:nix-community/nixpkgs.lib";
    atuin = {
      url = "github:atuinsh/atuin";
      inputs.flake-parts.flake = false;
    };
  };
  outputs = { self, ... }: { };
}
"#;
        assert_eq!(
            remove(FLAKE_FALSE_SHARED_PREFIX, "atuin.flake-parts.nixpkgs-lib"),
            expected
        );
    }

    #[test]
    fn remove_depth_two_follows_keeps_depth_one_follows_in_nested_attrset() {
        let expected = r#"{
  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    nixpkgs-lib.url = "github:nix-community/nixpkgs.lib";
    atuin = {
      url = "github:atuinsh/atuin";
      inputs = {
        flake-parts = {
          follows = "flake-parts";
        };
      };
    };
  };
  outputs = { self, ... }: { };
}
"#;
        assert_eq!(
            remove(
                NESTED_ATTRSET_SHARED_PREFIX,
                "atuin.flake-parts.nixpkgs-lib"
            ),
            expected
        );
    }

    #[test]
    fn remove_depth_two_follows_prunes_only_the_nested_attrset_it_empties() {
        let flake = r#"{
  inputs = {
    nixpkgs-lib.url = "github:nix-community/nixpkgs.lib";
    atuin = {
      url = "github:atuinsh/atuin";
      inputs = {
        flake-parts = {
          inputs.nixpkgs-lib.follows = "nixpkgs-lib";
        };
        flake-parts.flake = false;
      };
    };
  };
  outputs = { self, ... }: { };
}
"#;
        let expected = r#"{
  inputs = {
    nixpkgs-lib.url = "github:nix-community/nixpkgs.lib";
    atuin = {
      url = "github:atuinsh/atuin";
      inputs = {
        flake-parts.flake = false;
      };
    };
  };
  outputs = { self, ... }: { };
}
"#;
        assert_eq!(remove(flake, "atuin.flake-parts.nixpkgs-lib"), expected);
    }

    #[test]
    fn remove_input_drops_everything_it_owns_in_every_block_shape() {
        let expected = r#"{
  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    nixpkgs-lib.url = "github:nix-community/nixpkgs.lib";
  };
  outputs = { self, ... }: { };
}
"#;
        for flake in [
            INPUTS_ATTRSET_SHARED_PREFIX,
            FLAKE_FALSE_SHARED_PREFIX,
            NESTED_ATTRSET_SHARED_PREFIX,
        ] {
            assert_eq!(remove(flake, "atuin"), expected);
        }
    }

    /// An unrelated removal must not take out a self-named nested follows.
    #[test]
    fn unrelated_remove_keeps_self_named_nested_follows() {
        let flake = r#"{
  inputs = {
    themes.url = "github:o/themes";
    app.inputs.app.follows = "themes";
    other.inputs.sub.follows = "subtarget";
    subtarget.url = "github:o/subtarget";
  };

  outputs = { self, ... }: { };
}
"#;
        let change = Change::Remove {
            ids: vec![ChangeId::parse("other.sub").unwrap()],
        };
        let result = apply(flake, &change);

        assert!(
            !result.contains("other.inputs.sub.follows"),
            "the targeted follows should be removed, got:\n{result}"
        );
        assert!(
            result.contains("app.inputs.app.follows = \"themes\""),
            "a self-named follows for a different input must survive, got:\n{result}"
        );
    }

    #[test]
    fn sibling_remove_keeps_self_named_nested_follows() {
        let flake = r#"{
  inputs = {
    themes.url = "github:o/themes";
    app.url = "github:o/app";
    app.inputs.app.follows = "themes";
    app.inputs.foo.follows = "themes";
  };

  outputs = { self, ... }: { };
}
"#;
        let change = Change::Remove {
            ids: vec![ChangeId::parse("app.foo").unwrap()],
        };
        let result = apply(flake, &change);

        assert!(
            !result.contains("app.inputs.foo.follows"),
            "the targeted follows should be removed, got:\n{result}"
        );
        assert!(
            result.contains("app.inputs.app.follows = \"themes\""),
            "the self-named sibling follows must survive, got:\n{result}"
        );
    }

    #[test]
    fn sibling_remove_keeps_self_named_nested_follows_attrset() {
        let flake = r#"{
  inputs = {
    themes.url = "github:o/themes";
    app = {
      url = "github:o/app";
      inputs.app.follows = "themes";
      inputs.foo.follows = "themes";
    };
  };

  outputs = { self, ... }: { };
}
"#;
        let change = Change::Remove {
            ids: vec![ChangeId::parse("app.foo").unwrap()],
        };
        let result = apply(flake, &change);

        assert!(
            !result.contains("inputs.foo.follows"),
            "the targeted follows should be removed, got:\n{result}"
        );
        assert!(
            result.contains("inputs.app.follows = \"themes\""),
            "the self-named sibling follows must survive, got:\n{result}"
        );
    }

    #[test]
    fn targeted_remove_drops_self_named_nested_follows() {
        let flake = r#"{
  inputs = {
    themes.url = "github:o/themes";
    app.url = "github:o/app";
    app.inputs.app.follows = "themes";
  };

  outputs = { self, ... }: { };
}
"#;
        let change = Change::Remove {
            ids: vec![ChangeId::parse("app.app").unwrap()],
        };
        let result = apply(flake, &change);

        assert!(!result.contains("app.inputs.app.follows"), "got:\n{result}");
        assert!(result.contains("app.url"), "got:\n{result}");
    }

    #[test]
    fn whole_remove_drops_self_named_nested_follows() {
        let flake = r#"{
  inputs = {
    themes.url = "github:o/themes";
    app.url = "github:o/app";
    app.inputs.app.follows = "themes";
    app.inputs.nixpkgs.follows = "themes";
  };

  outputs = { self, ... }: { };
}
"#;
        let change = Change::Remove {
            ids: vec![ChangeId::parse("app").unwrap()],
        };
        let result = apply_until_fixed(flake, &change);

        assert!(!result.contains("app."), "got:\n{result}");
        assert!(result.contains("themes.url"), "got:\n{result}");
    }

    #[test]
    fn remove_input_keeps_same_named_nested_follows_of_other_input() {
        let flat = r#"{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
    nixpkgs-stable.url = "github:nixos/nixpkgs/nixos-25.05";
    crane.url = "github:ipetkov/crane";
    crane.inputs.nixpkgs.follows = "nixpkgs-stable";
  };
  outputs = { self, ... }: { };
}
"#;
        let attrset = r#"{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
    nixpkgs-stable.url = "github:nixos/nixpkgs/nixos-25.05";
    crane = {
      url = "github:ipetkov/crane";
      inputs.nixpkgs.follows = "nixpkgs-stable";
    };
  };
  outputs = { self, ... }: { };
}
"#;
        assert_eq!(
            remove(flat, "nixpkgs"),
            r#"{
  inputs = {
    nixpkgs-stable.url = "github:nixos/nixpkgs/nixos-25.05";
    crane.url = "github:ipetkov/crane";
    crane.inputs.nixpkgs.follows = "nixpkgs-stable";
  };
  outputs = { self, ... }: { };
}
"#
        );
        assert_eq!(
            remove(attrset, "nixpkgs"),
            r#"{
  inputs = {
    nixpkgs-stable.url = "github:nixos/nixpkgs/nixos-25.05";
    crane = {
      url = "github:ipetkov/crane";
      inputs.nixpkgs.follows = "nixpkgs-stable";
    };
  };
  outputs = { self, ... }: { };
}
"#
        );

        let toplevel = r#"{
  inputs.nixpkgs.url = "github:nixos/nixpkgs";
  inputs.nixpkgs-stable.url = "github:nixos/nixpkgs/nixos-25.05";
  inputs.crane.url = "github:ipetkov/crane";
  inputs.crane.inputs.nixpkgs = { follows = "nixpkgs-stable"; };
  outputs = { self, ... }: { };
}
"#;
        assert_eq!(
            remove(toplevel, "nixpkgs"),
            toplevel.replace("\n  inputs.nixpkgs.url = \"github:nixos/nixpkgs\";", "")
        );
    }

    #[test]
    fn nested_url_override_is_removed_with_owner_or_by_full_path() {
        let flat = r#"{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
    app.url = "github:o/app";
    app.inputs.foo.url = "github:o/foo";
  };
  outputs = { self, ... }: { };
}
"#;
        let attrset = r#"{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
    app = {
      url = "github:o/app";
      inputs.foo.url = "github:o/foo";
    };
  };
  outputs = { self, ... }: { };
}
"#;
        let removed = r#"{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
  };
  outputs = { self, ... }: { };
}
"#;
        assert_eq!(remove(flat, "app"), removed);
        assert_eq!(remove(attrset, "app"), removed);
        assert_eq!(
            remove(flat, "app.foo"),
            r#"{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
    app.url = "github:o/app";
  };
  outputs = { self, ... }: { };
}
"#
        );
        assert_eq!(
            remove(attrset, "app.foo"),
            r#"{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
    app = {
      url = "github:o/app";
    };
  };
  outputs = { self, ... }: { };
}
"#
        );
    }

    #[test]
    fn toplevel_nested_binding_is_removed_with_owner_or_by_full_path() {
        let flake = r#"{
  inputs.nixpkgs.url = "github:nixos/nixpkgs";
  inputs.app.url = "github:o/app";
  inputs.app.inputs.foo.url = "github:o/foo";
  inputs.nix.url = "github:NixOS/nix";
  inputs.nix.inputs.nixpkgs = { follows = "nixpkgs"; };
  inputs.e = { url = "github:o/e"; inputs = { nixpkgs = { follows = "nixpkgs"; }; }; };
  outputs = _: { };
}
"#;
        let without_override =
            flake.replace("\n  inputs.app.inputs.foo.url = \"github:o/foo\";", "");
        assert_eq!(remove(flake, "app.foo"), without_override);
        assert_eq!(
            remove(flake, "app"),
            without_override.replace("\n  inputs.app.url = \"github:o/app\";", "")
        );

        let without_follows = flake.replace(
            "\n  inputs.nix.inputs.nixpkgs = { follows = \"nixpkgs\"; };",
            "",
        );
        assert_eq!(remove(flake, "nix.nixpkgs"), without_follows);
        assert_eq!(
            remove(flake, "nix"),
            without_follows.replace("\n  inputs.nix.url = \"github:NixOS/nix\";", "")
        );

        assert_eq!(
            remove(flake, "e.nixpkgs"),
            flake.replace(" inputs = { nixpkgs = { follows = \"nixpkgs\"; }; };", "")
        );
    }

    #[test]
    fn block_follows_attrset_is_removed_with_owner_or_by_full_path() {
        let flake = r#"{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
    disko = {
      url = "github:nix-community/disko";
      inputs.nixpkgs = { follows = "nixpkgs"; };
    };
    hm.url = "github:nix-community/home-manager";
    hm.inputs.nixpkgs = { follows = "nixpkgs"; };
  };
  outputs = _: { };
}
"#;
        assert_eq!(
            remove(flake, "disko.nixpkgs"),
            flake.replace("\n      inputs.nixpkgs = { follows = \"nixpkgs\"; };", "")
        );
        let without_hm_follows =
            flake.replace("\n    hm.inputs.nixpkgs = { follows = \"nixpkgs\"; };", "");
        assert_eq!(remove(flake, "hm.nixpkgs"), without_hm_follows);
        assert_eq!(
            remove(flake, "hm"),
            without_hm_follows.replace("\n    hm.url = \"github:nix-community/home-manager\";", "")
        );
    }

    /// Nix resolves an owner declared only through a nested follows via the
    /// flake registry, so `flake:<owner>` names the same input.
    #[test]
    fn remove_follows_keeps_owner_declared_only_by_it() {
        let block = r#"{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
    disko.inputs.nixpkgs.follows = "nixpkgs";
  };
  outputs = inputs: { };
}
"#;
        assert_eq!(
            remove(block, "disko.nixpkgs"),
            r#"{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
    disko.url = "flake:disko";
  };
  outputs = inputs: { };
}
"#
        );

        let toplevel = r#"{
  inputs.nixpkgs.url = "github:nixos/nixpkgs";
  inputs.disko.inputs.nixpkgs.follows = "nixpkgs";
  outputs = inputs: { };
}
"#;
        assert_eq!(
            remove(toplevel, "disko.nixpkgs"),
            r#"{
  inputs.nixpkgs.url = "github:nixos/nixpkgs";
  inputs.disko.url = "flake:disko";
  outputs = inputs: { };
}
"#
        );
    }

    #[test]
    fn remove_follows_deletes_when_owner_stays_declared() {
        let flake = r#"{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
    disko.inputs.nixpkgs.follows = "nixpkgs";
    disko.inputs.systems.follows = "nixpkgs";
  };
  outputs = inputs: { };
}
"#;
        assert_eq!(
            remove(flake, "disko.nixpkgs"),
            r#"{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
    disko.inputs.systems.follows = "nixpkgs";
  };
  outputs = inputs: { };
}
"#
        );
        assert_eq!(
            remove(flake, "disko"),
            r#"{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
  };
  outputs = inputs: { };
}
"#
        );
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
    fn remove_input_strips_flat_depth_two_follows() {
        let flake = r#"{
  inputs = {
    nixpkgs-lib.url = "github:nix-community/nixpkgs.lib";
    atuin.url = "github:atuinsh/atuin";
    atuin.inputs.flake-parts.inputs.nixpkgs-lib.follows = "nixpkgs-lib";
    treefmt-nix.url = "github:numtide/treefmt-nix";
    treefmt-nix.inputs.flake-parts.inputs.nixpkgs-lib.follows = "nixpkgs-lib";
  };
  outputs = { self, ... }: { };
}
"#;
        let change = Change::Remove {
            ids: vec![ChangeId::parse("atuin").unwrap()],
        };
        let result = apply_until_fixed(flake, &change);

        assert_eq!(
            result,
            r#"{
  inputs = {
    nixpkgs-lib.url = "github:nix-community/nixpkgs.lib";
    treefmt-nix.url = "github:numtide/treefmt-nix";
    treefmt-nix.inputs.flake-parts.inputs.nixpkgs-lib.follows = "nixpkgs-lib";
  };
  outputs = { self, ... }: { };
}
"#
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
        let change = Change::Remove {
            ids: vec![ChangeId::parse("other").unwrap()],
        };
        let result =
            walk_children(&inputs_block, &None, &change).expect("Remove must rewrite the tree");
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
        let change = Change::Change {
            id: Some(ChangeId::parse("nixpkgs").unwrap()),
            uri: Some("github:NixOS/nixpkgs/nixos-23.11".to_string()),
        };
        let result =
            walk_children(&inputs_block, &None, &change).expect("Change must rewrite the tree");
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
        let change = Change::Add {
            id: Some(ChangeId::parse("nixpkgs").unwrap()),
            uri: Some("github:NixOS/nixpkgs/nixos-unstable".to_string()),
            flake: true,
        };
        let result =
            apply_add(inputs_block, &None, &change).expect("apply_add must rewrite the tree");
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
        let change = Change::Add {
            id: Some(ChangeId::parse("flake-utils").unwrap()),
            uri: Some("github:numtide/flake-utils".to_string()),
            flake: true,
        };
        let result =
            apply_add(inputs_block, &None, &change).expect("apply_add must rewrite the tree");
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
        let change = Change::Follows {
            input: ChangeId::parse("flake-edit.nixpkgs").unwrap(),
            target: AttrPath::parse("nixpkgs").unwrap(),
        };
        let result = apply_follows(inputs_block, &None, &change)
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
        let change = Change::Follows {
            input: ChangeId::parse("flake-edit").unwrap(),
            target: AttrPath::parse("nixpkgs").unwrap(),
        };
        let result = apply_follows(inputs_block, &None, &change)
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
    fn handle_url_leaf_returns_empty_for_matching_remove() {
        let flake = r#"{
  inputs = {
    nixpkgs = { url = "github:NixOS/nixpkgs/nixos-unstable"; };
  };

  outputs = { self, ... }: { };
}
"#;
        let (node, child, attr, leaf) = find_input_attrset_leaf(flake, "nixpkgs", "url");
        let change = Change::Remove {
            ids: vec![ChangeId::parse("nixpkgs").unwrap()],
        };
        let result = handle_url_leaf(&node, &child, &attr, &leaf, &change)
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
        let change = Change::Change {
            id: Some(ChangeId::parse("nixpkgs").unwrap()),
            uri: Some("github:NixOS/nixpkgs/nixos-23.11".to_string()),
        };
        let result = handle_url_leaf(&node, &child, &attr, &leaf, &change)
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
        let (node, child, _, _) = find_input_attrset_leaf(flake, "flake-edit", "inputs");
        let change = Change::Remove {
            ids: vec![ChangeId::parse("flake-edit.nixpkgs").unwrap()],
        };
        let result = handle_inputs_leaf(&node, &child, &change)
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
        let (node, child, _, _) = find_input_attrset_leaf(flake, "flake-edit", "inputs");
        let change = Change::Remove {
            ids: vec![ChangeId::parse("unrelated").unwrap()],
        };
        let result = handle_inputs_leaf(&node, &child, &change);
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
            &Segment::from_unquoted("flake-utils").unwrap(),
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
            &Segment::from_unquoted("naked").unwrap(),
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
            &Segment::from_unquoted("flake-utils").unwrap(),
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

    fn follows_change(input: &str, target: &str) -> Change {
        Change::Follows {
            input: ChangeId::parse(input).unwrap(),
            target: AttrPath::parse(target).unwrap(),
        }
    }

    #[test]
    fn follows_retargets_attrset_entry_of_inputs_block_before_url() {
        let flake = r#"{
  inputs = {
    a = {
      inputs = { t = { follows = "x"; }; };
      url = "github:o/a";
    };
    t.url = "github:o/t";
  };
  outputs = _: { };
}"#;
        assert_eq!(
            apply(flake, &follows_change("a.t", "t")),
            flake.replace(r#"follows = "x""#, r#"follows = "t""#)
        );
    }

    #[test]
    fn follows_retargets_nested_inputs_attrset_of_inputs_block() {
        let flake = r#"{
  inputs = {
    a = {
      url = "github:o/a";
      inputs = {
        n.inputs = { t.follows = ""; };
      };
    };
    t.url = "github:o/t";
  };
  outputs = _: { };
}"#;
        assert_eq!(
            apply(flake, &follows_change("a.n.t", "t")),
            flake.replace(r#"t.follows = """#, r#"t.follows = "t""#)
        );
    }

    #[test]
    fn follows_is_noop_for_split_declaration_with_same_target() {
        let flake = r#"{
  inputs = {
    a = {
      inputs.n = { inputs = { t = { follows = "t"; }; }; };
      url = "github:o/a";
    };
    t.url = "github:o/t";
  };
  outputs = _: { };
}"#;
        assert_eq!(apply(flake, &follows_change("a.n.t", "t")), flake);
    }
}
