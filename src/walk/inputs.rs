use std::collections::HashMap;

use rnix::{SyntaxKind, SyntaxNode};

use crate::change::Change;
use crate::input::Input;

use super::context::Context;
use super::node::{
    adjacent_whitespace_index, empty_node, get_sibling_whitespace, make_flake_false_attr,
    make_follows_attr, make_nested_follows_attr, make_quoted_string, make_toplevel_follows_attr,
    make_url_attr, parse_node, should_remove_input, should_remove_nested_input, substitute_child,
};

/// Remove a child node along with its adjacent whitespace.
pub fn remove_child_with_whitespace(
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
    id: String,
    input: Input,
    ctx: &Option<Context>,
) {
    if let Some(ctx) = ctx {
        if let Some(follows) = ctx.first() {
            if let Some(node) = inputs.get_mut(follows) {
                node.follows
                    .push(crate::input::Follows::Indirect(id, input.url));
                node.follows.sort();
                node.follows.dedup();
            } else {
                // In case the Input is not fully constructed
                let mut stub = Input::new(follows.to_string());
                stub.follows
                    .push(crate::input::Follows::Indirect(id, input.url));
                inputs.insert(follows.to_string(), stub);
            }
        }
    } else {
        // Update the input, in case there was already a stub present.
        if let Some(node) = inputs.get_mut(&id) {
            if !input.url.to_string().is_empty() {
                node.url = input.url;
            }
            if !input.flake {
                node.flake = input.flake;
            }
        } else {
            inputs.insert(id, input);
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
        let parent_id = input.input();
        let nested_id = input.follows();

        if let Some(nested_id) = nested_id {
            // Check if the parent input exists in this attr set (flat style)
            let parent_exists = inputs.contains_key(parent_id);

            // Check if this input uses nested-style (has an attr set block)
            // If it does, we should NOT add flat-style follows here - the nested
            // handler in handle_input_attr_set will add the follows inside the block
            let has_nested_block = node.children().any(|child| {
                // Look for `parent_id = { ... }` pattern
                if child.kind() != SyntaxKind::NODE_ATTRPATH_VALUE {
                    return false;
                }
                child
                    .first_child()
                    .and_then(|attrpath| attrpath.first_child())
                    .map(|first_ident| first_ident.to_string() == parent_id)
                    .unwrap_or(false)
                    && child
                        .children()
                        .any(|c| c.kind() == SyntaxKind::NODE_ATTR_SET)
            });

            // Only use flat-style if the input exists AND doesn't have a nested block
            if parent_exists && !has_nested_block {
                // Add a flat-style follows attribute to this attr set
                let follows_node = parse_node(&format!(
                    "{}.inputs.{}.follows = \"{}\";",
                    parent_id, nested_id, target
                ));

                // Find the last attribute belonging to this input's parent
                // This groups follows with their parent input instead of appending at the end
                let children: Vec<_> = node.children().collect();
                let insert_after = children.iter().rev().find(|child| {
                    // Check if this child's attrpath starts with parent_id
                    child
                        .first_child()
                        .and_then(|attrpath| attrpath.first_child())
                        .map(|first_ident| first_ident.to_string() == parent_id)
                        .unwrap_or(false)
                });

                // Use the found position, or fall back to end of inputs
                let reference_child = insert_after.or(children.last());
                if let Some(ref_child) = reference_child {
                    let insert_index = ref_child.index() + 1;

                    let mut green = node
                        .green()
                        .insert_child(insert_index, follows_node.green().into());

                    // Copy whitespace from before the reference child, but normalize it
                    // to a single newline + indentation (strip extra blank lines)
                    if let Some(whitespace) = get_sibling_whitespace(ref_child) {
                        let ws_str = whitespace.to_string();
                        // Keep only the last newline and subsequent indentation
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

        if nested_id.is_none() {
            // Only use flat-style if the input does not have a nested block.
            let has_nested_block = node.children().any(|child| {
                if child.kind() != SyntaxKind::NODE_ATTRPATH_VALUE {
                    return false;
                }
                child
                    .first_child()
                    .and_then(|attrpath| attrpath.first_child())
                    .map(|first_ident| first_ident.to_string() == parent_id)
                    .unwrap_or(false)
                    && child
                        .children()
                        .any(|c| c.kind() == SyntaxKind::NODE_ATTR_SET)
            });

            if !has_nested_block {
                let follows_node = make_toplevel_follows_attr(parent_id, target);

                // Find the last attribute belonging to this input's parent
                let children: Vec<_> = node.children().collect();
                let insert_after = children.iter().rev().find(|child| {
                    child
                        .first_child()
                        .and_then(|attrpath| attrpath.first_child())
                        .map(|first_ident| first_ident.to_string() == parent_id)
                        .unwrap_or(false)
                });

                let reference_child = insert_after.or(children.last());
                if let Some(ref_child) = reference_child {
                    let insert_index = ref_child.index() + 1;
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
    let id_str = input_id.to_string();
    let input = Input::with_url(id_str.clone(), url.to_string(), url.text_range());
    insert_with_ctx(inputs, id_str.clone(), input, ctx);

    if should_remove_input(change, ctx, &id_str) {
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
    let id_str = input_id.to_string();

    if should_remove_input(change, ctx, &id_str) {
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
    let id_str = input_id.to_string();

    for attr in nested_attr.children() {
        for binding in attr.children() {
            if binding.to_string() == "url" {
                let url = binding.next_sibling().unwrap();
                let input = Input::with_url(id_str.clone(), url.to_string(), input_id.text_range());
                insert_with_ctx(inputs, id_str.clone(), input, ctx);
            }
            if should_remove_input(change, ctx, &id_str) {
                return Some(empty_node());
            }
        }

        let context = id_str.clone().into();
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
        let context = id.to_string().into();
        if walk_inputs(inputs, child_node.clone(), &Some(context), change).is_some() {
            tracing::warn!(
                "Flat tree attribute replacement not yet implemented for: {}",
                child
            );
        }
    }

    None
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
        maybe_input_id.map(|id| id.to_string().into())
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
        let uri_node = make_url_attr(id, uri);
        let mut green = parent
            .green()
            .insert_child(child.index(), uri_node.green().into());

        if let Some(whitespace) = get_sibling_whitespace(child_node) {
            green = green.insert_child(child.index() + 1, whitespace.green().into());
        }

        if !flake {
            let no_flake = make_flake_false_attr(id);
            green = green.insert_child(child.index() + 2, no_flake.green().into());
            if let Some(whitespace) = get_sibling_whitespace(child_node) {
                green = green.insert_child(child.index() + 3, whitespace.green().into());
            }
        }
        return Some(parse_node(&green.to_string()));
    }

    None
}

/// Handle NODE_ATTRPATH nodes that represent "follows" attributes.
/// Example: `inputs.nixpkgs.follows = "nixpkgs"`
fn handle_attrpath_follows(
    inputs: &mut HashMap<String, Input>,
    node: &SyntaxNode,
    change: &Change,
) -> Option<SyntaxNode> {
    let maybe_follows_id = node
        .children()
        .find(|child| child.to_string() == "follows")
        .and_then(|input_child| input_child.prev_sibling());

    let follows_id = maybe_follows_id.as_ref()?;

    let maybe_input_id = node
        .children()
        .find(|child| child.to_string() == "inputs")
        .and_then(|input_child| input_child.next_sibling());

    let ctx = maybe_input_id.clone().map(|id| id.to_string().into());

    let url_node = node.next_sibling().unwrap();
    let input = Input::with_url(
        follows_id.to_string(),
        url_node.to_string(),
        url_node.text_range(),
    );
    insert_with_ctx(inputs, follows_id.to_string(), input, &ctx);

    // Remove a toplevel follows node
    if let Some(input_id) = maybe_input_id
        && change.is_remove()
        && let Some(id) = change.id()
    {
        let maybe_follows = maybe_follows_id.map(|id| id.to_string());
        if id.matches_with_follows(&input_id.to_string(), maybe_follows) {
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
        if let Change::Remove { ids } = change
            && ids.iter().any(|id| id.to_string() == prev_id.to_string())
        {
            return Some(empty_node());
        }
        if let Change::Change { id, uri, .. } = change
            && let Some(id) = id
            && *id == prev_id.to_string()
            && let Some(uri) = uri
            && let Some(url_node) = child.next_sibling()
        {
            let new_url = make_quoted_string(uri);
            return Some(substitute_child(node, url_node.index(), &new_url));
        }
        if let Some(sibling) = child.next_sibling() {
            let input = Input::with_url(
                prev_id.to_string(),
                sibling.to_string(),
                sibling.text_range(),
            );
            insert_with_ctx(inputs, prev_id.to_string(), input, ctx);
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
            let is_follows = nested_attr
                .first_child()
                .unwrap()
                .first_child()
                .unwrap()
                .next_sibling()
                .unwrap();

            if is_follows.to_string() == "follows" {
                let id = is_follows.prev_sibling().unwrap();
                let follows = nested_attr.first_child().unwrap().next_sibling().unwrap();
                let input =
                    Input::with_url(id.to_string(), follows.to_string(), follows.text_range());
                insert_with_ctx(inputs, id.to_string(), input, ctx);
                if should_remove_nested_input(change, ctx, &follows.to_string()) {
                    return Some(empty_node());
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
        let mut input = Input::new(input_id.to_string());
        input.flake = is_flake.to_string().parse().unwrap();
        let text_range = input_id.text_range();
        input.range = crate::input::Range::from_text_range(text_range);
        insert_with_ctx(inputs, input_id.to_string(), input, ctx);
        if should_remove_nested_input(change, ctx, &input_id.to_string()) {
            return Some(empty_node());
        }
    }
    None
}

/// Handle "follows" attribute within an input's ATTRPATH.
fn handle_follows_attr(
    inputs: &mut HashMap<String, Input>,
    attr: &SyntaxNode,
    ctx: &Option<Context>,
    change: &Change,
) -> Option<SyntaxNode> {
    let id = attr.prev_sibling().unwrap();
    let follows = attr.parent().unwrap().next_sibling().unwrap();
    let input = Input::with_url(id.to_string(), follows.to_string(), follows.text_range());
    insert_with_ctx(inputs, id.to_string(), input.clone(), ctx);
    if ctx.is_some() && should_remove_nested_input(change, ctx, input.id()) {
        return Some(empty_node());
    }
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
                let id = child.prev_sibling().unwrap();
                let uri = leaf.next_sibling().unwrap();
                let input = Input::with_url(id.to_string(), uri.to_string(), uri.text_range());
                insert_with_ctx(inputs, id.to_string(), input, ctx);

                // Remove matched node.
                if let Change::Remove { ids } = change
                    && ids
                        .iter()
                        .any(|candidate| candidate.to_string() == id.to_string())
                {
                    return Some(empty_node());
                }

                if let Change::Change {
                    id: Some(change_id),
                    uri: Some(new_uri),
                    ..
                } = change
                    && *change_id == id.to_string()
                {
                    let new_url = make_quoted_string(new_uri);
                    let new_attr =
                        substitute_child(&attr, leaf.next_sibling().unwrap().index(), &new_url);
                    let new_child = substitute_child(child, attr.index(), &new_attr);
                    return Some(substitute_child(node, child.index(), &new_child));
                }
            }

            if leaf.to_string().starts_with("inputs") {
                let id = child.prev_sibling().unwrap();
                let context = id.to_string().into();
                if let Some(replacement) =
                    walk_inputs(inputs, child.clone(), &Some(context), change)
                {
                    return Some(substitute_child(node, child.index(), &replacement));
                }
            }
        }
    }

    // Handle Change::Follows - add follows to this input's attr set
    if let Change::Follows { input, target } = change {
        let parent_id = input.input();
        let nested_id = input.follows();

        // Check if this attr set belongs to the input we want to modify
        if let Some(id_node) = child.prev_sibling()
            && id_node.to_string() == parent_id
        {
            if let Some(nested_id) = nested_id {
                // Insert the follows attribute into this attr set
                let follows_node = make_nested_follows_attr(nested_id, target);

                // Find the last actual child (before the closing brace)
                // and get whitespace from before it
                let children: Vec<_> = child.children().collect();
                if let Some(last_child) = children.last() {
                    // Insert after the last child with proper whitespace
                    let insert_index = last_child.index() + 1;

                    let mut green = child
                        .green()
                        .insert_child(insert_index, follows_node.green().into());

                    // Copy whitespace from before the last child
                    if let Some(whitespace) = get_sibling_whitespace(last_child) {
                        green = green.insert_child(insert_index, whitespace.green().into());
                    }

                    let new_child = parse_node(&green.to_string());
                    return Some(substitute_child(node, child.index(), &new_child));
                }
            } else {
                let has_follows = child.children().any(|attr| {
                    attr.first_child()
                        .and_then(|attrpath| attrpath.first_child())
                        .map(|first_ident| first_ident.to_string() == "follows")
                        .unwrap_or(false)
                });

                if !has_follows {
                    let follows_node = make_follows_attr(target);
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
