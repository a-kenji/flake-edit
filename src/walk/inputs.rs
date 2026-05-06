use std::collections::HashMap;

use rnix::{SyntaxKind, SyntaxNode};

use crate::change::Change;
use crate::follows::{AttrPath, Segment, strip_outer_quotes};
use crate::input::Input;

use super::context::Context;
use super::node::{
    FollowsKind, adjacent_whitespace_index, empty_node, get_sibling_whitespace,
    insertion_index_after, is_attrset_content_empty, make_attrset_url_attr,
    make_attrset_url_flake_false_attr, make_flake_false_attr, make_quoted_string, make_url_attr,
    parse_node, should_remove_input, should_remove_nested_input, substitute_child,
};

/// Sentinel segment used when CST text cannot form a valid [`Segment`].
const INVALID_SEGMENT_SENTINEL: &str = "__invalid__";

/// Read a CST node as an unquoted [`Segment`].
///
/// Falls back to [`INVALID_SEGMENT_SENTINEL`] when the node text would be
/// rejected by [`Segment::from_unquoted`], so the walker keeps making forward
/// progress against malformed input instead of panicking. Emits a
/// `tracing::warn!` so the fall-through is observable.
fn segment_from_syntax(node: &SyntaxNode) -> Segment {
    Segment::from_syntax(node).unwrap_or_else(|err| {
        let raw = node.to_string();
        tracing::warn!(
            "walk/inputs: invalid attribute segment {raw:?} ({err}); using sentinel \
             {INVALID_SEGMENT_SENTINEL:?}"
        );
        Segment::from_unquoted(INVALID_SEGMENT_SENTINEL)
            .expect("sentinel segment is non-empty and quote-free")
    })
}

/// Parse the right-hand side of a `follows = "..."` binding into a typed
/// target.
///
/// Empty input produces `None`, the in-flake analog of the lockfile's
/// `Input::Indirect(None)` (`inputs.X = []`). Non-empty input is split
/// on `/`, the only separator Nix recognises in a follows target. A `.`
/// inside a segment is part of the identifier (`"hls-1.10/nixpkgs"` is
/// two segments, not three). Each segment passes through
/// [`Segment::from_unquoted`]; if the body is malformed the result
/// falls back to a single-segment path built from `fallback_seg` so
/// the walker never loses an entry.
fn parse_follows_target(text: &str, fallback_seg: &Segment) -> Option<AttrPath> {
    if text.is_empty() {
        return None;
    }
    let body = strip_outer_quotes(text);
    if body.is_empty() {
        return None;
    }
    let mut segs = body
        .split('/')
        .filter(|s| !s.is_empty())
        .filter_map(|s| Segment::from_unquoted(s.to_string()).ok());
    let Some(first) = segs.next() else {
        return Some(AttrPath::new(fallback_seg.clone()));
    };
    let mut path = AttrPath::new(first);
    for seg in segs {
        path.push(seg);
    }
    Some(path)
}

/// Remove `node` from `parent` along with any adjacent whitespace token.
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
            let target = parse_follows_target(&input.url, &id);
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
pub fn walk_inputs(
    inputs: &mut HashMap<String, Input>,
    node: SyntaxNode,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    match node.kind() {
        SyntaxKind::NODE_ATTRPATH => {
            if let Some(result) = handle_attrpath_follows(inputs, &node, change) {
                return Some(result);
            }
        }
        SyntaxKind::NODE_ATTR_SET | SyntaxKind::NODE_ATTRPATH_VALUE | SyntaxKind::NODE_IDENT => {}
        _ => {}
    }

    // Add follows on flat-style inputs declared inside an `inputs = { ... }`
    // attr set, when the parent input has no nested `{ ... }` block.
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
                    .map(|first_ident| {
                        strip_outer_quotes(&first_ident.to_string()) == parent_id_str
                    })
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
                // block - `Walker::handle_follows_flat_toplevel` owns the
                // split-declaration placement instead.
                let children: Vec<_> = node.children().collect();
                let insert_after = children.iter().rev().find(|child| {
                    child
                        .first_child()
                        .and_then(|attrpath| attrpath.first_child())
                        .map(|first_ident| {
                            strip_outer_quotes(&first_ident.to_string()) == parent_id_str
                        })
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
                    .map(|first_ident| {
                        strip_outer_quotes(&first_ident.to_string()) == parent_id_str
                    })
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
                        .map(|first_ident| {
                            strip_outer_quotes(&first_ident.to_string()) == parent_id_str
                        })
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

    // Add an entry into an empty `inputs = { }` attr set.
    // Indentation comes from the whitespace preceding the `inputs`
    // attrpath-value node. Contents indent one level deeper.
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

        // Drop any whitespace already sitting between the braces, then
        // rebuild the contents from scratch.
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

        return Some(parse_node(&green.to_string()));
    }

    None
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

/// Handle a flat-style flake attribute (`inputs.foo.flake = false`), returning
/// the replacement node when `change` removes the input.
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

/// Handle a nested input declaration (`inputs.foo = { url = "..."; ... }`),
/// returning the replacement node when `change` modifies it.
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

/// Whether `parent`'s input declarations predominantly use attrset style
/// (`foo = { url = "..."; };`) over flat style (`foo.url = "...";`).
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

/// Indent slice of a whitespace token: everything after the last `\n`.
/// For `"\n    "` returns `"    "`.
fn extract_indent(ws_str: &str) -> &str {
    if let Some(last_nl) = ws_str.rfind('\n') {
        &ws_str[last_nl + 1..]
    } else {
        ws_str
    }
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
        maybe_input_id.map(|id| segment_from_syntax(&id).into())
    } else {
        ctx.clone()
    };

    if let Some(replacement) = walk_input(inputs, child_node, &ctx, change) {
        let mut green = parent
            .green()
            .replace_child(child.index(), replacement.green().into());

        // Strip adjacent whitespace when the child was removed outright.
        if replacement.text().is_empty()
            && let Some(ws_index) = adjacent_whitespace_index(child)
        {
            green = green.remove_child(ws_index);
        }
        return Some(parse_node(&green.to_string()));
    }

    // Add a new entry into a non-empty `inputs = { ... }` block.
    if ctx.is_none()
        && let Change::Add {
            id: Some(id),
            uri: Some(uri),
            flake,
        } = change
    {
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
    let children: Vec<SyntaxNode> = node.children().collect();
    let last = children.last()?;
    if last.to_string() != "follows" {
        return None;
    }

    // First non-`inputs` segment owns the follows. Remaining non-`inputs`
    // segments form the nested path beneath it.
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
        // Toplevel direct follows: `inputs.<owner>.follows = "T"`. Store the
        // target text as a synthetic url on the owning input so depth-1
        // ctx-driven flows keep working, and honor a `Change::Remove`
        // matching this owner.
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

    // Build the [`crate::input::Follows::Indirect`] directly so we can carry
    // the full multi-segment chain. `insert_with_ctx`'s single-segment path
    // would truncate intermediate segments here.
    let target = parse_follows_target(&leaf_input.url, &follows_seg);
    let mut path_iter = rest.iter().cloned();
    let mut nested_path = AttrPath::new(path_iter.next().expect("rest non-empty"));
    for seg in path_iter {
        nested_path.push(seg);
    }

    let key = owner_seg.as_str().to_string();
    let entry = inputs
        .entry(key)
        .or_insert_with(|| Input::new(owner_seg.clone()));
    entry.push_indirect_follows(nested_path, target);

    // Remove the follows node when the change targets it. Depth-1 uses the
    // existing two-segment matcher; depth-N walks the full
    // [`crate::change::ChangeId`] path because
    // [`crate::change::ChangeId::input`] / [`crate::change::ChangeId::follows`]
    // only expose the first two segments.
    if change.is_remove()
        && let Some(id) = change.id()
    {
        if rest.len() == 1 {
            let owner_match_seg = segment_from_syntax(&owner_node);
            if id.matches_with_follows(&owner_match_seg, Some(&follows_seg)) {
                return Some(empty_node());
            }
        } else if rest.len() >= 2 {
            let id_segs = id.path().segments();
            if id_segs.len() == rest.len() + 1
                && id_segs[0] == owner_seg
                && id_segs[1..].iter().zip(rest.iter()).all(|(a, b)| a == b)
            {
                return Some(empty_node());
            }
        }
    }

    None
}

/// Handle a `url = ...` binding inside an input's attrset.
fn handle_url_attr(
    inputs: &mut HashMap<String, Input>,
    node: &SyntaxNode,
    child: &SyntaxNode,
    attr: &SyntaxNode,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    if let Some(prev_id) = attr.prev_sibling() {
        // `inputs.X.url = "..."` inside another input's attrset is a
        // transitive URL override, not a follows. With ctx set,
        // `insert_with_ctx` would read the URL string as a follows target.
        if ctx.is_some() {
            return None;
        }
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

    // Nested follows that live as siblings of the `url` attribute.
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
                // Flat attrpath style: `nixpkgs.follows = "nixpkgs"`.
                if follows_ident.to_string() == "follows" {
                    let id_seg = segment_from_syntax(&first_ident);
                    let Some(follows) = attrpath.next_sibling() else {
                        continue;
                    };
                    let input =
                        Input::with_url(id_seg.clone(), follows.to_string(), follows.text_range());
                    insert_with_ctx(inputs, id_seg, input, ctx);
                    let follows_target_seg = segment_from_syntax(&follows);
                    if should_remove_nested_input(change, ctx, &follows_target_seg) {
                        return Some(empty_node());
                    }
                }
            } else if let Some(value_node) = attrpath.next_sibling()
                && SyntaxKind::NODE_ATTR_SET == value_node.kind()
            {
                // Nested attrset style: `nixpkgs = { follows = "nixpkgs"; }`.
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
                        let follows_target_seg = segment_from_syntax(&follows);
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

    // Discriminator: the raw attrpath's first ident is `inputs` for the
    // inside-an-input-block shape (chain is the nested path under the
    // ctx-supplied owner), and the owner ident otherwise (chain[0] is the
    // owner and must be stripped to produce the nested path).
    let attrpath_starts_with_inputs = attrpath
        .children()
        .next()
        .map(|c| c.to_string() == "inputs")
        .unwrap_or(false);
    let chain_first = segment_from_syntax(&chain[0]);
    let (owner_seg, nested_segs): (Segment, Vec<Segment>) =
        match ctx.as_ref().and_then(|c| c.first().cloned()) {
            Some(ctx_owner) if attrpath_starts_with_inputs => {
                (ctx_owner, chain.iter().map(segment_from_syntax).collect())
            }
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
    let target = parse_follows_target(unquoted, &leaf_seg);

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

/// Expected attrpath idents for a follows lookup inside a parent input's
/// `{ ... }` block: `inputs.<R0>.inputs.<R1>...inputs.<RN>.follows`.
fn block_follows_idents(rest: &[Segment]) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::with_capacity(rest.len() * 2 + 1);
    for seg in rest.iter() {
        out.push("inputs");
        out.push(seg.as_str());
    }
    out.push("follows");
    out
}

/// Expected attrpath idents for a flat follows lookup outside a parent's
/// block (e.g. inside `inputs = { ... }`): `<S0>.inputs.<S1>...inputs.<SN>.follows`.
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

/// Locate a flat `<S0>.inputs.<S1>...inputs.<SN>.follows` attr at `node`'s
/// level (e.g. inside an `inputs = { ... }` block) and rebuild it the same
/// way as [`find_existing_nested_follows`].
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

/// Handle a `NODE_ATTR_SET` inside an input node, processing nested
/// `url` and `inputs` bindings.
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

                // Removal inside a nested inputs attrset:
                // `inputs = { nixpkgs = { follows = "nixpkgs"; }; }`.
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

                            // Prune the now-empty `inputs = { ... }` binding
                            // when it became content-empty as a consequence
                            // of this removal. Comments inside the block are
                            // user-authored content and suppress pruning.
                            //
                            // The pruning is bounded to the `inputs` binding:
                            // the input's own outer block (e.g. `disko = { url
                            // = "..."; inputs = { ... }; }`) keeps its url and
                            // any other siblings intact even if the binding
                            // itself disappears.
                            let new_child = if is_attrset_content_empty(&new_inputs_attrset) {
                                remove_child_with_whitespace(child, &attr, attr.index())
                            } else {
                                let new_attr = substitute_child(
                                    &attr,
                                    inputs_attrset.index(),
                                    &new_inputs_attrset,
                                );
                                substitute_child(child, attr.index(), &new_attr)
                            };

                            return Some(substitute_child(node, child.index(), &new_child));
                        }
                    }
                }
            }
        }
    }

    if let Change::Follows { input, target } = change {
        let full_path = input.path();
        let parent_id = input.input();
        let parent_id_str = parent_id.as_str();
        let target_str = target.to_string();

        if let Some(id_node) = child.prev_sibling()
            && strip_outer_quotes(&id_node.to_string()) == parent_id_str
        {
            // Inside the parent's `{ ... }` block, emit the chain relative
            // to the parent (everything below it).
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

#[cfg(test)]
mod tests {
    use crate::change::{Change, ChangeId};
    use crate::walk::Walker;

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
    fn parse_follows_target_accepts_slash_form() {
        use crate::follows::Segment;
        let fallback = Segment::from_unquoted("fallback").unwrap();
        let parsed = super::parse_follows_target("hyprland/hyprlang", &fallback)
            .expect("non-empty input must parse to Some");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed.first().as_str(), "hyprland");
        assert_eq!(parsed.last().as_str(), "hyprlang");
    }

    #[test]
    fn parse_follows_target_dot_inside_segment_is_not_a_separator() {
        use crate::follows::Segment;
        let fallback = Segment::from_unquoted("fallback").unwrap();

        let single = super::parse_follows_target("hls-1.10", &fallback).unwrap();
        assert_eq!(single.len(), 1);
        assert_eq!(single.first().as_str(), "hls-1.10");

        let two = super::parse_follows_target("hls-1.10/nixpkgs", &fallback).unwrap();
        assert_eq!(two.len(), 2);
        assert_eq!(two.first().as_str(), "hls-1.10");
        assert_eq!(two.last().as_str(), "nixpkgs");
    }

    #[test]
    fn segment_from_syntax_falls_back_to_sentinel_on_empty_string() {
        use rnix::SyntaxKind;

        // An empty quoted attribute (`""`) parses but its segment text is
        // empty, so `Segment::from_source` rejects it and the walker has to
        // substitute the sentinel to keep traversing.
        let src = r#"{ inputs."" = {}; }"#;
        let parsed = rnix::Root::parse(src);
        fn find_first_string(node: rnix::SyntaxNode) -> Option<rnix::SyntaxNode> {
            if node.kind() == SyntaxKind::NODE_STRING {
                return Some(node);
            }
            for c in node.children() {
                if let Some(s) = find_first_string(c) {
                    return Some(s);
                }
            }
            None
        }
        let empty_string = find_first_string(parsed.syntax()).expect("CST has an empty string");
        let seg = super::segment_from_syntax(&empty_string);
        assert_eq!(seg.as_str(), super::INVALID_SEGMENT_SENTINEL);
    }
}
