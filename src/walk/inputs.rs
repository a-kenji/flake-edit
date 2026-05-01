use std::collections::HashMap;

use rnix::{SyntaxKind, SyntaxNode};

use crate::change::Change;
use crate::follows::{AttrPath, Segment};
use crate::input::Input;

use super::context::Context;
use super::node::{
    FollowsKind, adjacent_whitespace_index, empty_node, get_sibling_whitespace,
    insertion_index_after, make_attrset_url_attr, make_attrset_url_flake_false_attr,
    make_flake_false_attr, make_quoted_string, make_url_attr, parse_node, should_remove_input,
    should_remove_nested_input, substitute_child,
};

/// Read a CST node as an unquoted [`Segment`]. Falls back to a sentinel
/// segment if the node text contains characters [`Segment::from_unquoted`]
/// rejects (so the walker keeps making forward progress against malformed
/// input rather than panicking).
fn segment_from_syntax(node: &SyntaxNode) -> Segment {
    Segment::from_syntax(node).unwrap_or_else(|_| {
        Segment::from_unquoted("__invalid__").expect("sentinel segment is non-empty and quote-free")
    })
}

/// Strip outer `"..."` from a CST string token's text.
fn unquote(s: &str) -> &str {
    s.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(s)
}

/// Remove a child node along with its adjacent whitespace.
fn remove_child_with_whitespace(
    parent: &SyntaxNode,
    node: &SyntaxNode,
    index: usize,
) -> SyntaxNode {
    let mut green = parent.green().remove_child(index);
    let element: rnix::SyntaxElement = node.clone().into();
    if let Some(ws_index) = adjacent_whitespace_index(&element) {
        green = green.remove_child(ws_index);
    }
    parse_node(&green.to_string())
}

/// Insert a new Input node at the correct position or update it with new information.
pub fn insert_with_ctx(
    inputs: &mut HashMap<String, Input>,
    id: Segment,
    input: Input,
    ctx: &Option<Context>,
) {
    if let Some(ctx) = ctx {
        if let Some(follows) = ctx.first() {
            // The follows target arrives as the `input.url` token; parse it
            // as a path so downstream comparisons are structural, falling
            // back to a literal single-segment path on parse failure.
            let target = AttrPath::parse(&input.url)
                .or_else(|_| Segment::from_unquoted(input.url.clone()).map(AttrPath::new))
                .unwrap_or_else(|_| AttrPath::new(id.clone()));
            let key = follows.as_str().to_string();
            // `id` is a single segment because this entry-point only sees
            // depth-1 follows shapes (`<owner>.<id>.follows = ...`). The deep
            // case is parsed up front in `handle_attrpath_follows` and
            // bypasses this helper.
            let nested_path = AttrPath::new(id.clone());
            if let Some(node) = inputs.get_mut(&key) {
                node.follows.push(crate::input::Follows::Indirect {
                    path: nested_path,
                    target,
                });
                node.follows.sort();
                node.follows.dedup();
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

/// Walk the inputs section of a flake.nix file.
pub fn walk_inputs(
    inputs: &mut HashMap<String, Input>,
    node: SyntaxNode,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    // Handle special node types at the top level
    match node.kind() {
        SyntaxKind::NODE_ATTRPATH => {
            if let Some(result) = handle_attrpath_follows(inputs, &node, change) {
                return Some(result);
            }
        }
        SyntaxKind::NODE_ATTR_SET | SyntaxKind::NODE_ATTRPATH_VALUE | SyntaxKind::NODE_IDENT => {}
        _ => {}
    }

    // Handle Change::Follows for flat-style inputs (inputs inside an attr set)
    // This adds follows to the inputs attr set when there's no nested block
    if let Change::Follows { input, target } = change
        && node.kind() == SyntaxKind::NODE_ATTR_SET
        && ctx.is_none()
    {
        let full_path = input.path();
        let parent_id = input.input();
        let parent_id_str = parent_id.as_str();
        let target_str = target.to_string();

        if full_path.len() >= 2 {
            let parent_exists = inputs.contains_key(parent_id_str);

            let has_nested_block = node.children().any(|child| {
                if child.kind() != SyntaxKind::NODE_ATTRPATH_VALUE {
                    return false;
                }
                child
                    .first_child()
                    .and_then(|attrpath| attrpath.first_child())
                    .map(|first_ident| unquote(&first_ident.to_string()) == parent_id_str)
                    .unwrap_or(false)
                    && child
                        .children()
                        .any(|c| c.kind() == SyntaxKind::NODE_ATTR_SET)
            });

            if parent_exists && !has_nested_block {
                if let Some(result) =
                    find_existing_flat_follows(&node, full_path.segments(), &target_str)
                {
                    return result;
                }

                let follows_node = FollowsKind::InputsBlockNested {
                    path: full_path,
                    target: &target_str,
                }
                .emit();

                // Bail out (None) when the parent isn't declared in this
                // block - `walk_toplevel`'s `handle_follows_flat_toplevel`
                // owns the split-declaration placement instead.
                let children: Vec<_> = node.children().collect();
                let insert_after = children.iter().rev().find(|child| {
                    child
                        .first_child()
                        .and_then(|attrpath| attrpath.first_child())
                        .map(|first_ident| unquote(&first_ident.to_string()) == parent_id_str)
                        .unwrap_or(false)
                });

                if let Some(ref_child) = insert_after {
                    let insert_index = insertion_index_after(ref_child);

                    let mut green = node
                        .green()
                        .insert_child(insert_index, follows_node.green().into());

                    if let Some(whitespace) = get_sibling_whitespace(ref_child) {
                        let ws_str = whitespace.to_string();
                        let normalized = if let Some(last_nl) = ws_str.rfind('\n') {
                            &ws_str[last_nl..]
                        } else {
                            &ws_str
                        };
                        let ws_node = parse_node(normalized);
                        green = green.insert_child(insert_index, ws_node.green().into());
                    }

                    return Some(parse_node(&green.to_string()));
                }
            }
        }

        if full_path.len() == 1 {
            let has_nested_block = node.children().any(|child| {
                if child.kind() != SyntaxKind::NODE_ATTRPATH_VALUE {
                    return false;
                }
                child
                    .first_child()
                    .and_then(|attrpath| attrpath.first_child())
                    .map(|first_ident| unquote(&first_ident.to_string()) == parent_id_str)
                    .unwrap_or(false)
                    && child
                        .children()
                        .any(|c| c.kind() == SyntaxKind::NODE_ATTR_SET)
            });

            if !has_nested_block {
                let follows_node = FollowsKind::TopLevelFlat {
                    id: parent_id,
                    target: &target_str,
                }
                .emit();

                let children: Vec<_> = node.children().collect();
                let insert_after = children.iter().rev().find(|child| {
                    child
                        .first_child()
                        .and_then(|attrpath| attrpath.first_child())
                        .map(|first_ident| unquote(&first_ident.to_string()) == parent_id_str)
                        .unwrap_or(false)
                });

                if let Some(ref_child) = insert_after {
                    let insert_index = insertion_index_after(ref_child);
                    let mut green = node
                        .green()
                        .insert_child(insert_index, follows_node.green().into());

                    if let Some(whitespace) = get_sibling_whitespace(ref_child) {
                        let ws_str = whitespace.to_string();
                        let normalized = if let Some(last_nl) = ws_str.rfind('\n') {
                            &ws_str[last_nl..]
                        } else {
                            &ws_str
                        };
                        let ws_node = parse_node(normalized);
                        green = green.insert_child(insert_index, ws_node.green().into());
                    }

                    return Some(parse_node(&green.to_string()));
                }
            }
        }
    }

    for child in node.children_with_tokens() {
        match child.kind() {
            SyntaxKind::NODE_ATTRPATH_VALUE => {
                if let Some(result) =
                    handle_child_attrpath_value(inputs, &node, &child, ctx, change)
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

    // Handle Add into an empty `inputs = { }` attr set.
    // Derive indentation from the whitespace preceding the parent `inputs`
    // attrpath-value node, then indent contents one level deeper.
    if node.kind() == SyntaxKind::NODE_ATTR_SET
        && ctx.is_none()
        && let Change::Add {
            id: Some(id),
            uri: Some(uri),
            flake,
        } = change
        && !node
            .children()
            .any(|c| c.kind() == SyntaxKind::NODE_ATTRPATH_VALUE)
    {
        // Derive indentation: look at the whitespace before the parent
        // `inputs = { }` node to get the base indent, then add one level.
        let base_indent = node
            .parent()
            .and_then(|p| p.prev_sibling_or_token())
            .filter(|t| t.kind() == SyntaxKind::TOKEN_WHITESPACE)
            .map(|t| {
                let ws = t.to_string();
                ws.rfind('\n')
                    .map(|i| &ws[i + 1..])
                    .unwrap_or(&ws)
                    .to_string()
            })
            .unwrap_or_else(|| "  ".to_string());
        let entry_indent = format!("\n{}  ", base_indent);
        let closing_indent = format!("\n{}", base_indent);

        let uri_node = make_url_attr(id, uri);

        // Remove any existing whitespace between braces in the empty set,
        // then rebuild from a clean string representation.
        let ws_index = node
            .children_with_tokens()
            .find(|t| t.kind() == SyntaxKind::TOKEN_WHITESPACE)
            .map(|t| t.index());

        let mut green = if let Some(idx) = ws_index {
            node.green().remove_child(idx)
        } else {
            node.green().into_owned()
        };

        // Find closing brace position after potential whitespace removal
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

        // Add closing indent before the brace
        green = green.insert_child(
            brace_index + offset,
            parse_node(&closing_indent).green().into(),
        );

        return Some(parse_node(&green.to_string()));
    }

    None
}

/// Handle flat-style URL attribute: `inputs.foo.url = "..."`
/// Returns Some(node) if a modification was made.
fn handle_flat_url(
    inputs: &mut HashMap<String, Input>,
    input_id: &SyntaxNode,
    url: &SyntaxNode,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    let id_seg = segment_from_syntax(input_id);
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
        && *change_id == id_str
    {
        return Some(make_quoted_string(new_uri));
    }

    None
}

/// Handle flat-style flake attribute: `inputs.foo.flake = false`
/// Returns Some(node) if a modification was made.
fn handle_flat_flake(
    input_id: &SyntaxNode,
    _is_flake: &SyntaxNode,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    let id_seg = segment_from_syntax(input_id);

    if should_remove_input(change, ctx, &id_seg) {
        return Some(empty_node());
    }

    None
}

/// Handle nested input attributes like `inputs.foo = { url = "..."; ... }`
/// Returns Some(node) if a modification was made.
fn handle_nested_input(
    inputs: &mut HashMap<String, Input>,
    input_id: &SyntaxNode,
    nested_attr: &SyntaxNode,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    let id_seg = segment_from_syntax(input_id);

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

/// Handle a NODE_IDENT child node during input walking.
/// Processes flat-style input declarations like `inputs.nixpkgs.url = "..."`
/// Returns Some(node) if a modification was made, None otherwise.
fn handle_child_ident(
    inputs: &mut HashMap<String, Input>,
    child: &rnix::SyntaxElement,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    let child_node = child.as_node()?;
    let parent_sibling = child_node.parent().and_then(|p| p.next_sibling());

    // Handle "inputs" identifier with next sibling
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

    // Handle flat tree attributes like "inputs.X.Y"
    if child.to_string().starts_with("inputs") {
        let id = child_node.next_sibling()?;
        let context: Context = segment_from_syntax(&id).into();
        if walk_inputs(inputs, child_node.clone(), &Some(context), change).is_some() {
            tracing::warn!(
                "Flat tree attribute replacement not yet implemented for: {}",
                child
            );
        }
    }

    None
}

/// Detect whether inputs predominantly use attrset style (`foo = { url = "..."; };`)
/// or flat style (`foo.url = "...";`).
/// Returns true if attrset style is more common among input declarations.
fn uses_attrset_style(parent: &SyntaxNode) -> bool {
    let mut attrset_count = 0usize;
    let mut flat_url_count = 0usize;

    for child in parent.children() {
        if child.kind() != SyntaxKind::NODE_ATTRPATH_VALUE {
            continue;
        }

        if child
            .children()
            .any(|c| c.kind() == SyntaxKind::NODE_ATTR_SET)
        {
            attrset_count += 1;
            continue;
        }

        if let Some(attrpath) = child
            .children()
            .find(|c| c.kind() == SyntaxKind::NODE_ATTRPATH)
        {
            let idents: Vec<_> = attrpath.children().collect();
            if idents.len() >= 2
                && idents
                    .last()
                    .map(|i| i.to_string() == "url")
                    .unwrap_or(false)
            {
                flat_url_count += 1;
            }
        }
    }

    attrset_count > flat_url_count
}

/// Extract the indentation string from a whitespace node.
/// Given whitespace like `\n    `, returns `    ` (everything after the last newline).
fn extract_indent(ws_str: &str) -> &str {
    if let Some(last_nl) = ws_str.rfind('\n') {
        &ws_str[last_nl + 1..]
    } else {
        ws_str
    }
}

/// Handle a NODE_ATTRPATH_VALUE child node during input walking.
/// Returns Some(node) if a modification was made, None otherwise.
fn handle_child_attrpath_value(
    inputs: &mut HashMap<String, Input>,
    parent: &SyntaxNode,
    child: &rnix::SyntaxElement,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    let child_node = child.as_node().unwrap();

    // Build context if not present
    let ctx = if ctx.is_none() {
        let maybe_input_id = child_node.children().find_map(|c| {
            c.children()
                .find(|child| child.to_string() == "inputs")
                .and_then(|input_child| input_child.prev_sibling())
        });
        maybe_input_id.map(|id| segment_from_syntax(&id).into())
    } else {
        ctx.clone()
    };

    // Try to walk the input and apply changes
    if let Some(replacement) = walk_input(inputs, child_node, &ctx, change) {
        let mut green = parent
            .green()
            .replace_child(child.index(), replacement.green().into());

        // Remove adjacent whitespace if the replacement is empty
        if replacement.text().is_empty()
            && let Some(ws_index) = adjacent_whitespace_index(child)
        {
            green = green.remove_child(ws_index);
        }
        return Some(parse_node(&green.to_string()));
    }

    // Handle Add change when no context exists
    if ctx.is_none()
        && let Change::Add {
            id: Some(id),
            uri: Some(uri),
            flake,
        } = change
    {
        // Find the last NODE_ATTRPATH_VALUE child to append after it
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

        // Use whitespace from before the last input, but normalize to a single
        // newline + indentation.  Copying the raw inter-entry whitespace would
        // duplicate blank lines when the closing brace already has one.
        let ws_reference = last_attr.as_ref().unwrap_or(child_node);
        if let Some(whitespace) = get_sibling_whitespace(ws_reference) {
            let ws_str = whitespace.to_string();
            let normalized = if let Some(last_nl) = ws_str.rfind('\n') {
                &ws_str[last_nl..]
            } else {
                &ws_str
            };
            let ws_node = parse_node(normalized);
            let mut green = parent
                .green()
                .insert_child(insert_index, ws_node.green().into());
            let mut offset = 1;

            if use_attrset {
                let indent = extract_indent(&ws_str);
                let uri_node = if *flake {
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
                    let compact_ws = if let Some(last_nl) = ws_str.rfind('\n') {
                        &ws_str[last_nl..]
                    } else {
                        &ws_str
                    };
                    let compact_ws_node = parse_node(compact_ws);
                    green =
                        green.insert_child(insert_index + offset, compact_ws_node.green().into());
                    offset += 1;
                    green = green.insert_child(insert_index + offset, no_flake.green().into());
                }
            }
            return Some(parse_node(&green.to_string()));
        }

        let uri_node = make_url_attr(id, uri);
        let mut green = parent
            .green()
            .insert_child(insert_index, uri_node.green().into());

        if !flake {
            let no_flake = make_flake_false_attr(id);
            green = green.insert_child(insert_index + 1, no_flake.green().into());
        }
        return Some(parse_node(&green.to_string()));
    }

    None
}

/// Handle NODE_ATTRPATH nodes that represent "follows" attributes.
///
/// Recognizes any depth of nested-follows shape, e.g.
///
/// ```nix
/// inputs.A.follows = "T";                              # depth 0 (toplevel)
/// inputs.A.inputs.B.follows = "T";                     # depth 1
/// inputs.A.inputs.B.inputs.C.follows = "T";            # depth 2
/// A.inputs.B.inputs.C.follows = "T";                   # block-flat depth 2
/// ```
///
/// The owning input is the first non-`inputs` segment; the remaining
/// non-`inputs` segments (excluding the trailing `follows`) form the
/// nested-input path stored on the owner via [`Follows::Indirect`].
fn handle_attrpath_follows(
    inputs: &mut HashMap<String, Input>,
    node: &SyntaxNode,
    change: &Change,
) -> Option<SyntaxNode> {
    let children: Vec<SyntaxNode> = node.children().collect();
    let last = children.last()?;
    if last.to_string() != "follows" {
        return None;
    }

    // First non-`inputs` segment is the owning input; the rest is the nested
    // path under it.
    let path_segments: Vec<(SyntaxNode, Segment)> = children[..children.len() - 1]
        .iter()
        .filter(|c| c.to_string() != "inputs")
        .map(|c| (c.clone(), segment_from_syntax(c)))
        .collect();

    let owner_node = path_segments.first()?.0.clone();
    let owner_seg = path_segments.first()?.1.clone();
    let rest: Vec<Segment> = path_segments[1..].iter().map(|(_, s)| s.clone()).collect();

    let url_node = node.next_sibling()?;

    if rest.is_empty() {
        // Top-level direct follows: `inputs.<owner>.follows = "T"`. Store
        // the target text as a synthetic url on the owning input so
        // depth-1 ctx-driven flows continue to work as before, and honor
        // a `Change::Remove` matching this owner.
        let input = Input::with_url(
            owner_seg.clone(),
            url_node.to_string(),
            url_node.text_range(),
        );
        insert_with_ctx(inputs, owner_seg.clone(), input, &None);
        if change.is_remove()
            && let Some(id) = change.id()
            && id.matches_with_follows(&owner_seg, Some(&owner_seg))
        {
            return Some(empty_node());
        }
        return None;
    }

    let follows_seg = rest.last().cloned().expect("rest is non-empty");
    let leaf_input = Input::with_url(
        follows_seg.clone(),
        url_node.to_string(),
        url_node.text_range(),
    );

    // Build the `Follows::Indirect` directly so we can carry the full
    // multi-segment chain. `insert_with_ctx`'s single-segment helper would
    // truncate intermediate segments here.
    let target = AttrPath::parse(&leaf_input.url)
        .or_else(|_| Segment::from_unquoted(leaf_input.url.clone()).map(AttrPath::new))
        .unwrap_or_else(|_| AttrPath::new(follows_seg.clone()));
    let mut path_iter = rest.iter().cloned();
    let mut nested_path = AttrPath::new(path_iter.next().expect("rest non-empty"));
    for seg in path_iter {
        nested_path.push(seg);
    }

    let key = owner_seg.as_str().to_string();
    let entry = inputs
        .entry(key)
        .or_insert_with(|| Input::new(owner_seg.clone()));
    entry.follows.push(crate::input::Follows::Indirect {
        path: nested_path,
        target,
    });
    entry.follows.sort();
    entry.follows.dedup();

    // Remove a toplevel follows node when the change targets it. The match
    // is depth-1 only.
    if change.is_remove()
        && let Some(id) = change.id()
        && rest.len() == 1
    {
        let owner_match_seg = segment_from_syntax(&owner_node);
        if id.matches_with_follows(&owner_match_seg, Some(&follows_seg)) {
            return Some(empty_node());
        }
    }

    None
}

/// Handle "url" attribute within an input's ATTRPATH.
fn handle_url_attr(
    inputs: &mut HashMap<String, Input>,
    node: &SyntaxNode,
    child: &SyntaxNode,
    attr: &SyntaxNode,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    if let Some(prev_id) = attr.prev_sibling() {
        let prev_seg = segment_from_syntax(&prev_id);
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
            && *id == prev_str
            && let Some(uri) = uri
            && let Some(url_node) = child.next_sibling()
        {
            let new_url = make_quoted_string(uri);
            return Some(substitute_child(node, url_node.index(), &new_url));
        }
        if let Some(sibling) = child.next_sibling() {
            let input =
                Input::with_url(prev_seg.clone(), sibling.to_string(), sibling.text_range());
            insert_with_ctx(inputs, prev_seg, input, ctx);
        }
    }

    // Handle nested follows within url attribute
    if let Some(parent) = child.parent()
        && let Some(sibling) = parent.next_sibling()
        && let Some(nested_child) = sibling.first_child()
        && nested_child.to_string() == "inputs"
        && let Some(attr_set) = nested_child.next_sibling()
        && SyntaxKind::NODE_ATTR_SET == attr_set.kind()
    {
        for nested_attr in attr_set.children() {
            let Some(attrpath) = nested_attr.first_child() else {
                continue;
            };
            let Some(first_ident) = attrpath.first_child() else {
                continue;
            };

            if let Some(follows_ident) = first_ident.next_sibling() {
                // Flat attrpath style: `nixpkgs.follows = "nixpkgs"`
                if follows_ident.to_string() == "follows" {
                    let id_seg = segment_from_syntax(&first_ident);
                    let Some(follows) = attrpath.next_sibling() else {
                        continue;
                    };
                    let input =
                        Input::with_url(id_seg.clone(), follows.to_string(), follows.text_range());
                    insert_with_ctx(inputs, id_seg, input, ctx);
                    let follows_target_seg =
                        Segment::from_unquoted(unquote(&follows.to_string()).to_string())
                            .unwrap_or_else(|_| Segment::from_unquoted("__invalid__").unwrap());
                    if should_remove_nested_input(change, ctx, &follows_target_seg) {
                        return Some(empty_node());
                    }
                }
            } else if let Some(value_node) = attrpath.next_sibling()
                && SyntaxKind::NODE_ATTR_SET == value_node.kind()
            {
                // Deeply nested attrset style: `nixpkgs = { follows = "nixpkgs"; }`
                let id_seg = segment_from_syntax(&first_ident);
                for inner_attr in value_node.children() {
                    let Some(inner_path) = inner_attr.first_child() else {
                        continue;
                    };
                    let Some(inner_ident) = inner_path.first_child() else {
                        continue;
                    };
                    if inner_ident.to_string() == "follows" {
                        let Some(follows) = inner_path.next_sibling() else {
                            continue;
                        };
                        let input = Input::with_url(
                            id_seg.clone(),
                            follows.to_string(),
                            follows.text_range(),
                        );
                        insert_with_ctx(inputs, id_seg.clone(), input, ctx);
                        let follows_target_seg =
                            Segment::from_unquoted(unquote(&follows.to_string()).to_string())
                                .unwrap_or_else(|_| Segment::from_unquoted("__invalid__").unwrap());
                        if should_remove_nested_input(change, ctx, &follows_target_seg) {
                            return Some(empty_node());
                        }
                    }
                }
            }
        }
    }
    None
}

/// Handle "flake" attribute within an input's ATTRPATH.
fn handle_flake_attr(
    inputs: &mut HashMap<String, Input>,
    attr: &SyntaxNode,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    if let Some(input_id) = attr.prev_sibling()
        && let Some(is_flake) = attr.parent().unwrap().next_sibling()
    {
        let id_seg = segment_from_syntax(&input_id);
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

/// Handle "follows" attribute within an input's ATTRPATH.
///
/// Supports any depth of nesting. The parent attrpath's idents are walked,
/// the literal `inputs` keywords are stripped, and the surviving idents
/// form the source chain. The owning input comes from `ctx` when present
/// (the inside-an-input-block case `inputs.X.follows = ...` inside `crane =
/// { ... }`), or from `chain[0]` otherwise (the inputs-block-flat case
/// `crane.inputs.X.follows = ...`).
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

    // The attrpath shape decides where the owner lives: at the inputs-block
    // surface (`crane.inputs.X...`) the chain starts with the owner segment;
    // inside a parent's `{ ... }` block (`inputs.X...`) the chain is already
    // the nested-path under the ctx-supplied owner.
    let chain_first = segment_from_syntax(&chain[0]);
    let (owner_seg, nested_segs): (Segment, Vec<Segment>) =
        match ctx.as_ref().and_then(|c| c.first().cloned()) {
            Some(ctx_owner) if ctx_owner == chain_first => (
                ctx_owner,
                chain[1..].iter().map(segment_from_syntax).collect(),
            ),
            Some(ctx_owner) => (ctx_owner, chain.iter().map(segment_from_syntax).collect()),
            None => (
                chain_first,
                chain[1..].iter().map(segment_from_syntax).collect(),
            ),
        };

    if nested_segs.len() <= 1 {
        // Depth-1 keeps the historic single-segment storage and remove-flow;
        // the depth-N branch below builds a typed multi-segment path.
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
    let unquoted = url_text
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(&url_text);
    let leaf_seg = nested_path.last().clone();
    let target = AttrPath::parse(unquoted)
        .or_else(|_| Segment::from_unquoted(unquoted.to_string()).map(AttrPath::new))
        .unwrap_or_else(|_| AttrPath::new(leaf_seg));

    let key = owner_seg.as_str().to_string();
    let entry = inputs
        .entry(key)
        .or_insert_with(|| Input::new(owner_seg.clone()));
    entry.follows.push(crate::input::Follows::Indirect {
        path: nested_path,
        target,
    });
    entry.follows.sort();
    entry.follows.dedup();

    None
}

/// Handle NODE_ATTRPATH within an input node.
/// Dispatches to url, flake, and follows handlers.
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

/// Outcome of a follows-attr lookup: which `NODE_ATTRPATH_VALUE` child carries
/// the matching `... = "<target>";` attribute, the value node (when present),
/// and whether the existing target already matches.
struct ExistingFollows {
    attr: SyntaxNode,
    value: Option<SyntaxNode>,
    same_target: bool,
}

/// Locate a follows attr inside `container` whose attrpath idents - compared
/// pairwise with surrounding `"..."` quotes stripped - equal `expected`.
///
/// `expected` is a slice of unquoted idents covering the entire attrpath
/// (e.g. `["inputs", "nixpkgs", "follows"]` or `["crane", "inputs",
/// "nixpkgs", "follows"]`). The caller decides how to splice the
/// replacement back into the surrounding tree.
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
            .all(|(have, want)| unquote(have) == *want)
        {
            continue;
        }
        let value = attrpath.next_sibling();
        let current_target = value
            .as_ref()
            .map(|v| unquote(&v.to_string()).to_string())
            .unwrap_or_default();
        return Some(ExistingFollows {
            attr,
            value,
            same_target: current_target == target,
        });
    }
    None
}

/// Build the expected attrpath idents for a follows lookup inside a
/// parent input's `{ ... }` block: `inputs.<R0>.inputs.<R1>...inputs.<RN>.follows`.
fn block_follows_idents(rest: &[Segment]) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::with_capacity(rest.len() * 2 + 1);
    for seg in rest.iter() {
        out.push("inputs");
        out.push(seg.as_str());
    }
    out.push("follows");
    out
}

/// Build the expected attrpath idents for a flat follows lookup outside
/// a parent's block (e.g. inside `inputs = { ... }`):
/// `<S0>.inputs.<S1>...inputs.<SN>.follows`.
fn flat_follows_idents(path: &[Segment]) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::with_capacity(path.len() * 2);
    for (i, seg) in path.iter().enumerate() {
        if i > 0 {
            out.push("inputs");
        }
        out.push(seg.as_str());
    }
    out.push("follows");
    out
}

/// Look up an `inputs.<R0>...inputs.<RN>.follows` attr inside an input's
/// `{ ... }` block (descending into intermediate `inputs = { ... }` blocks
/// implicitly via flat-attrpath matching) and produce the rebuilt outer
/// `node`. Returns the unchanged outer node on a same-target hit, the
/// retargeted outer node on a different-target hit, or `None` if no match.
fn find_existing_nested_follows(
    node: &SyntaxNode,
    attr_set: &SyntaxNode,
    rest: &[Segment],
    target: &str,
) -> Option<Option<SyntaxNode>> {
    let expected = block_follows_idents(rest);
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

/// Look up a flat `<S0>.inputs.<S1>...inputs.<SN>.follows` attr at `node`'s
/// level (e.g. inside an `inputs = { ... }` block) and rebuild the same way.
fn find_existing_flat_follows(
    node: &SyntaxNode,
    path: &[Segment],
    target: &str,
) -> Option<Option<SyntaxNode>> {
    let expected = flat_follows_idents(path);
    let found = find_existing_follows(node, &expected, target)?;
    if found.same_target {
        return Some(Some(node.clone()));
    }
    let value = found.value?;
    let new_value = make_quoted_string(target);
    let new_attr = substitute_child(&found.attr, value.index(), &new_value);
    Some(Some(substitute_child(node, found.attr.index(), &new_attr)))
}

/// Handle NODE_ATTR_SET within an input node.
/// Processes nested attribute sets containing url and inputs.
fn handle_input_attr_set(
    inputs: &mut HashMap<String, Input>,
    node: &SyntaxNode,
    child: &SyntaxNode,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    for attr in child.children() {
        for leaf in attr.children() {
            if leaf.to_string() == "url" {
                let id_node = child.prev_sibling().unwrap();
                let id_seg = segment_from_syntax(&id_node);
                let id_str = id_seg.as_str().to_string();
                let uri = leaf.next_sibling().unwrap();
                let input = Input::with_url(id_seg.clone(), uri.to_string(), uri.text_range());
                insert_with_ctx(inputs, id_seg.clone(), input, ctx);

                if let Change::Remove { ids } = change
                    && ids.iter().any(|candidate| {
                        candidate.input().as_str() == id_str && candidate.follows().is_none()
                    })
                {
                    return Some(empty_node());
                }

                if let Change::Change {
                    id: Some(change_id),
                    uri: Some(new_uri),
                    ..
                } = change
                    && *change_id == id_str
                {
                    let new_url = make_quoted_string(new_uri);
                    let new_attr =
                        substitute_child(&attr, leaf.next_sibling().unwrap().index(), &new_url);
                    let new_child = substitute_child(child, attr.index(), &new_attr);
                    return Some(substitute_child(node, child.index(), &new_child));
                }
            }

            if leaf.to_string().starts_with("inputs") {
                let id_node = child.prev_sibling().unwrap();
                let id_seg = segment_from_syntax(&id_node);
                let context: Context = id_seg.clone().into();
                let ctx_some = Some(context);
                if let Some(replacement) = walk_inputs(inputs, child.clone(), &ctx_some, change) {
                    return Some(substitute_child(node, child.index(), &replacement));
                }

                // Handle deeply nested inputs attrset removal:
                // `inputs = { nixpkgs = { follows = "nixpkgs"; }; }`
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
                        let nested_seg = segment_from_syntax(&nested_id);
                        if should_remove_nested_input(change, &ctx_some, &nested_seg) {
                            let new_inputs_attrset = remove_child_with_whitespace(
                                &inputs_attrset,
                                &nested_entry,
                                nested_entry.index(),
                            );
                            let new_attr = substitute_child(
                                &attr,
                                inputs_attrset.index(),
                                &new_inputs_attrset,
                            );
                            let new_child = substitute_child(child, attr.index(), &new_attr);
                            return Some(substitute_child(node, child.index(), &new_child));
                        }
                    }
                }
            }
        }
    }

    // Handle Change::Follows - add follows to this input's attr set
    if let Change::Follows { input, target } = change {
        let full_path = input.path();
        let parent_id = input.input();
        let parent_id_str = parent_id.as_str();
        let target_str = target.to_string();

        if let Some(id_node) = child.prev_sibling()
            && unquote(&id_node.to_string()) == parent_id_str
        {
            // Inside the parent's `{ ... }` block, emit the relative chain
            // (everything below the parent segment).
            let rest: Vec<Segment> = full_path.segments()[1..].to_vec();
            if !rest.is_empty() {
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
                    let insert_index = last_child.index() + 1;

                    let mut green = child
                        .green()
                        .insert_child(insert_index, follows_node.green().into());

                    if let Some(whitespace) = get_sibling_whitespace(last_child) {
                        green = green.insert_child(insert_index, whitespace.green().into());
                    }

                    let new_child = parse_node(&green.to_string());
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

                        let new_child = parse_node(&green.to_string());
                        return Some(substitute_child(node, child.index(), &new_child));
                    }
                }
            }
        }
    }

    None
}

/// Walk a single input field.
/// Example:
/// ```nix
///  flake-utils.url = "github:numtide/flake-utils";
/// ```
/// or
/// ```nix
///  rust-overlay = {
///  url = "github:oxalica/rust-overlay";
///  inputs.nixpkgs.follows = "nixpkgs";
///  inputs.flake-utils.follows = "flake-utils";
///  };
/// ```
pub fn walk_input(
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
