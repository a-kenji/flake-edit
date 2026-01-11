use std::collections::HashMap;

use rnix::{Root, SyntaxKind, SyntaxNode};
use thiserror::Error;

use crate::{
    change::Change,
    edit::{OutputChange, Outputs},
    input::Input,
};

// Type alias for clearer function signatures
type Node = SyntaxNode;

/// Errors that can occur during AST walking and manipulation.
#[derive(Debug, Error)]
pub enum WalkerError {
    #[error("Expected root node, found {0:?}")]
    NotARoot(SyntaxKind),

    #[error("Expected {expected:?}, found {found:?}")]
    UnexpectedNodeKind {
        expected: SyntaxKind,
        found: SyntaxKind,
    },

    #[error("Feature not yet implemented: {0}")]
    NotImplemented(String),
}

/// Parse a string into a SyntaxNode.
fn parse_node(s: &str) -> Node {
    Root::parse(s).syntax()
}

/// Create an empty syntax node, used when removing nodes.
fn empty_node() -> Node {
    Root::parse("").syntax()
}

/// Get a whitespace node copied from adjacent siblings, if present.
/// Checks previous sibling first, then next sibling.
fn get_sibling_whitespace(node: &SyntaxNode) -> Option<Node> {
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

/// Create a quoted string node.
/// Example: `"github:NixOS/nixpkgs"`
fn make_quoted_string(s: &str) -> Node {
    parse_node(&format!("\"{}\"", s))
}

/// Create a toplevel input URL attribute node.
/// Example: `inputs.nixpkgs.url = "github:NixOS/nixpkgs";`
fn make_toplevel_url_attr(id: &str, uri: &str) -> Node {
    parse_node(&format!("inputs.{}.url = \"{}\";", id, uri))
}

/// Create a toplevel input flake=false attribute node.
/// Example: `inputs.not_a_flake.flake = false;`
fn make_toplevel_flake_false_attr(id: &str) -> Node {
    parse_node(&format!("inputs.{}.flake = false;", id))
}

/// Create a nested input URL attribute node.
/// Example: `nixpkgs.url = "github:NixOS/nixpkgs";`
fn make_url_attr(id: &str, uri: &str) -> Node {
    parse_node(&format!("{}.url = \"{}\";", id, uri))
}

/// Create a nested input flake=false attribute node.
/// Example: `not_a_flake.flake = false;`
fn make_flake_false_attr(id: &str) -> Node {
    parse_node(&format!("{}.flake = false;", id))
}

/// Remove whitespace adjacent to a child element from a green node.
/// Used after removing or replacing a child with an empty node.
fn strip_whitespace_after_child(
    green: rowan::GreenNode,
    child: &rnix::SyntaxElement,
) -> rowan::GreenNode {
    let mut green = green;
    if let Some(prev) = child.prev_sibling_or_token()
        && prev.kind() == SyntaxKind::TOKEN_WHITESPACE
    {
        green = green.remove_child(prev.index());
    } else if let Some(next) = child.next_sibling_or_token()
        && next.kind() == SyntaxKind::TOKEN_WHITESPACE
    {
        green = green.remove_child(next.index());
    }
    green
}

/// Check if an input should be removed based on the change and context.
fn should_remove_input(change: &Change, ctx: &Option<Context>, input_id: &str) -> bool {
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

#[derive(Debug, Clone)]
pub struct Walker {
    pub root: SyntaxNode,
    pub inputs: HashMap<String, Input>,
    pub add_toplevel: bool,
}

#[derive(Debug, Clone)]
/// A helper for the [`Walker`], in order to hold context while traversing the tree.
pub struct Context {
    level: Vec<String>,
}

impl Context {
    /// Returns the first (top) level of context, if any.
    pub fn first(&self) -> Option<&str> {
        self.level.first().map(|s| s.as_str())
    }

    /// Returns true if the first level matches the given string.
    pub fn first_matches(&self, s: &str) -> bool {
        self.first() == Some(s)
    }

    pub fn level(&self) -> &[String] {
        &self.level
    }
}

impl From<String> for Context {
    fn from(s: String) -> Self {
        Self { level: vec![s] }
    }
}

impl<'a> Walker {
    pub fn new(stream: &'a str) -> Self {
        let root = Root::parse(stream).syntax();
        Self {
            root,
            inputs: HashMap::new(),
            add_toplevel: false,
        }
    }
    /// Traverse the toplevel `flake.nix` file.
    /// It should consist of three attribute keys:
    /// - description
    /// - inputs
    /// - outputs
    pub fn walk(&mut self, change: &Change) -> Result<Option<SyntaxNode>, WalkerError> {
        let cst = &self.root;
        if cst.kind() != SyntaxKind::NODE_ROOT {
            return Err(WalkerError::NotARoot(cst.kind()));
        }
        self.walk_toplevel(cst.clone(), None, change)
    }
    /// Insert a new Input node at the correct position
    /// or update it with new information.
    fn insert_with_ctx(&mut self, id: String, input: Input, ctx: &Option<Context>) {
        tracing::debug!("Inserting id: {id}, input: {input:?} with: ctx: {ctx:?}");

        if let Some(ctx) = ctx {
            // TODO: add more nesting
            if let Some(follows) = ctx.first() {
                if let Some(node) = self.inputs.get_mut(follows) {
                    // TODO: only indirect follows is handled
                    node.follows
                        .push(crate::input::Follows::Indirect(id, input.url));
                    // TODO: this should not be necessary
                    node.follows.sort();
                    node.follows.dedup();
                } else {
                    // In case the Input is not fully constructed
                    let mut stub = Input::new(follows.to_string());
                    stub.follows
                        .push(crate::input::Follows::Indirect(id, input.url));
                    self.inputs.insert(follows.to_string(), stub);
                }
            }
        } else {
            // Update the input, in case there was already a stub present.
            if let Some(node) = self.inputs.get_mut(&id) {
                if !input.url.to_string().is_empty() {
                    node.url = input.url;
                }
                if !input.flake {
                    node.flake = input.flake;
                }
            } else {
                self.inputs.insert(id, input);
            }
        }
        tracing::debug!("Self Inputs: {:#?}", self.inputs);
    }

    fn remove_child_with_whitespace(
        parent: &SyntaxNode,
        node: &SyntaxNode,
        index: usize,
    ) -> SyntaxNode {
        let green = parent.green().remove_child(index);
        let element: rnix::SyntaxElement = node.clone().into();
        let green = strip_whitespace_after_child(green, &element);
        parse_node(&green.to_string())
    }

    /// Only walk the outputs attribute
    pub(crate) fn list_outputs(&mut self) -> Result<Outputs, WalkerError> {
        let mut outputs: Vec<String> = vec![];
        let mut any = false;
        tracing::debug!("Walking outputs.");
        let cst = &self.root;
        if cst.kind() != SyntaxKind::NODE_ROOT {
            return Err(WalkerError::NotARoot(cst.kind()));
        }

        for toplevel in cst.first_child().unwrap().children() {
            if toplevel.kind() == SyntaxKind::NODE_ATTRPATH_VALUE {
                {
                    if let Some(outputs_node) = toplevel
                        .children()
                        .find(|child| child.to_string() == "outputs")
                    {
                        assert!(outputs_node.kind() == SyntaxKind::NODE_ATTRPATH);

                        if let Some(outputs_lambda) = outputs_node.next_sibling() {
                            assert!(outputs_lambda.kind() == SyntaxKind::NODE_LAMBDA);
                            if let Some(output) = outputs_lambda
                                .children()
                                .find(|n| n.kind() == SyntaxKind::NODE_PATTERN)
                            {
                                // We need to iterate over tokens, because ellipsis ...
                                // is not a valid node itself.
                                for child in output.children_with_tokens() {
                                    if child.kind() == SyntaxKind::NODE_PAT_ENTRY {
                                        outputs.push(child.to_string());
                                    }
                                    if child.kind() == SyntaxKind::TOKEN_ELLIPSIS {
                                        any = true;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        if outputs.is_empty() {
            Ok(Outputs::None)
        } else if any {
            Ok(Outputs::Any(outputs))
        } else {
            Ok(Outputs::Multiple(outputs))
        }
    }
    /// Only change the outputs attribute
    pub(crate) fn change_outputs(
        &mut self,
        change: OutputChange,
    ) -> Result<Option<SyntaxNode>, WalkerError> {
        tracing::debug!("Changing outputs.");
        let cst = &self.root;
        if cst.kind() != SyntaxKind::NODE_ROOT {
            return Err(WalkerError::NotARoot(cst.kind()));
        }

        for toplevel in cst.first_child().unwrap().children() {
            if toplevel.kind() == SyntaxKind::NODE_ATTRPATH_VALUE {
                {
                    if let Some(outputs_node) = toplevel
                        .children()
                        .find(|child| child.to_string() == "outputs")
                    {
                        assert!(outputs_node.kind() == SyntaxKind::NODE_ATTRPATH);

                        if let Some(outputs_lambda) = outputs_node.next_sibling() {
                            assert!(outputs_lambda.kind() == SyntaxKind::NODE_LAMBDA);
                            for output in outputs_lambda.children() {
                                if SyntaxKind::NODE_PATTERN == output.kind() {
                                    if let OutputChange::Add(ref add) = change {
                                        let token_count = output.children_with_tokens().count();
                                        let count = output.children().count();
                                        let last_node = token_count - 2;

                                        // Adjust the addition for trailing slasheks
                                        let addition = if let Some(SyntaxKind::TOKEN_COMMA) = output
                                            .children()
                                            .last()
                                            .and_then(|last| last.next_sibling_or_token())
                                            .map(|last_token| last_token.kind())
                                        {
                                            parse_node(&format!("{add},"))
                                        } else {
                                            parse_node(&format!(", {add}"))
                                        };

                                        let mut green = output
                                            .green()
                                            .insert_child(last_node, addition.green().into());
                                        if let Some(prev) = output
                                            .children()
                                            .nth(count - 1)
                                            .unwrap()
                                            .prev_sibling_or_token()
                                            && let SyntaxKind::TOKEN_WHITESPACE = prev.kind()
                                        {
                                            let whitespace =
                                                parse_node(prev.as_token().unwrap().green().text());
                                            green = green
                                                .insert_child(last_node, whitespace.green().into());
                                        }
                                        let changed_outputs_lambda = outputs_lambda
                                            .green()
                                            .replace_child(output.index(), green.into());
                                        let changed_toplevel = toplevel.green().replace_child(
                                            outputs_lambda.index(),
                                            changed_outputs_lambda.into(),
                                        );
                                        let result =
                                            cst.first_child().unwrap().green().replace_child(
                                                toplevel.index(),
                                                changed_toplevel.into(),
                                            );
                                        return Ok(Some(parse_node(&result.to_string())));
                                    }

                                    for child in output.children() {
                                        if child.kind() == SyntaxKind::NODE_PAT_ENTRY
                                            && let OutputChange::Remove(ref id) = change
                                            && child.to_string() == *id
                                        {
                                            let mut green =
                                                output.green().remove_child(child.index());
                                            if let Some(prev) = child.prev_sibling_or_token() {
                                                if let SyntaxKind::TOKEN_WHITESPACE = prev.kind() {
                                                    green = green.remove_child(prev.index());
                                                    green = green.remove_child(prev.index() - 1);
                                                }
                                            } else if let Some(next) = child.next_sibling_or_token()
                                                && let SyntaxKind::TOKEN_WHITESPACE = next.kind()
                                            {
                                                green = green.remove_child(next.index());
                                            }
                                            let changed_outputs_lambda = outputs_lambda
                                                .green()
                                                .replace_child(output.index(), green.into());
                                            let changed_toplevel = toplevel.green().replace_child(
                                                outputs_lambda.index(),
                                                changed_outputs_lambda.into(),
                                            );
                                            let result =
                                                cst.first_child().unwrap().green().replace_child(
                                                    toplevel.index(),
                                                    changed_toplevel.into(),
                                                );
                                            return Ok(Some(parse_node(&result.to_string())));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(None)
    }
    /// Traverse the toplevel `flake.nix` file.
    /// It should consist of three attribute keys:
    /// - description
    /// - inputs
    /// - outputs
    #[allow(clippy::option_map_unit_fn)]
    fn walk_toplevel(
        &mut self,
        node: SyntaxNode,
        ctx: Option<Context>,
        change: &Change,
    ) -> Result<Option<SyntaxNode>, WalkerError> {
        for root in node.children() {
            // Because it is the node root this is the toplevel attribute
            for toplevel in root.children() {
                // Match attr_sets inputs, and outputs
                tracing::debug!("Toplevel: {}", toplevel);
                tracing::debug!("Kind: {:?}", toplevel.kind());
                if toplevel.kind() == SyntaxKind::NODE_ATTRPATH_VALUE {
                    for child in toplevel.children() {
                        tracing::debug!("Toplevel Child: {child}");
                        tracing::debug!("Toplevel Child Kind: {:?}", child.kind());
                        tracing::debug!("Toplevel Child Index: {:?}", child.index());
                        tracing::debug!("Toplevel Index: {:?}", toplevel.index());
                        if let Some(parent) = child.parent() {
                            tracing::debug!("Toplevel Child Parent: {}", parent);
                            tracing::debug!("Toplevel Child Parent Kind: {:?}", parent.kind());
                            tracing::debug!("Toplevel Child Parent Index: {:?}", parent.index());
                        }
                        if child.to_string() == "description" {
                            // We are not interested in the description
                            break;
                        }
                        if child.to_string() == "inputs" {
                            if let Some(replacement) =
                                self.walk_inputs(child.next_sibling().unwrap(), &ctx, change)
                            {
                                tracing::debug!("Replacement Node: {replacement}");
                                let green = toplevel.green().replace_child(
                                    child.next_sibling().unwrap().index(),
                                    replacement.green().into(),
                                );
                                let green = toplevel.replace_with(green);
                                return Ok(Some(parse_node(&green.to_string())));
                            }
                        } else if child.to_string().starts_with("inputs") {
                            // This is a toplevel node, of the form:
                            // input.id ...
                            // If the node should be empty,
                            // it's toplevel should be empty too.
                            if let Some(replacement) = self.walk_inputs(child.clone(), &ctx, change)
                            {
                                if replacement.to_string().is_empty() {
                                    let node = Self::remove_child_with_whitespace(
                                        &root,
                                        &toplevel,
                                        toplevel.index(),
                                    );
                                    return Ok(Some(node));
                                } else {
                                    tracing::debug!("Replacement Node: {replacement}");
                                    let green = toplevel.green().replace_child(
                                        child.next_sibling().unwrap().index(),
                                        replacement.green().into(),
                                    );
                                    let green = toplevel.replace_with(green);
                                    return Ok(Some(parse_node(&green.to_string())));
                                }
                            }
                        };
                        // If we already see outputs, but have no inputs
                        // we need to create a toplevel inputs attribute set.
                        if child.to_string() == "outputs"
                            && self.add_toplevel
                            && let Change::Add {
                                id: Some(id),
                                uri: Some(uri),
                                flake,
                            } = change
                        {
                            let addition = make_toplevel_url_attr(id, uri);
                            // TODO Guard against indices that would be out of range here.
                            if toplevel.index() > 0 {
                                let mut node = root
                                    .green()
                                    .insert_child(toplevel.index() - 1, addition.green().into());
                                if let Some(c) =
                                    root.children().find(|c| c.index() == toplevel.index() - 2)
                                    && let Some(whitespace) = get_sibling_whitespace(&c)
                                {
                                    node = node.insert_child(
                                        toplevel.index() - 1,
                                        whitespace.green().into(),
                                    );
                                }
                                if !flake {
                                    let no_flake = make_toplevel_flake_false_attr(id);
                                    node = node.insert_child(
                                        toplevel.index() + 1,
                                        no_flake.green().into(),
                                    );
                                    if let Some(c) =
                                        root.children().find(|c| c.index() == toplevel.index() - 2)
                                        && let Some(whitespace) = get_sibling_whitespace(&c)
                                    {
                                        node = node.insert_child(
                                            toplevel.index() + 1,
                                            whitespace.green().into(),
                                        );
                                    }
                                }
                                if let Some(prev) = child.next_sibling_or_token()
                                    && prev.kind() == SyntaxKind::TOKEN_WHITESPACE
                                {
                                    let whitespace =
                                        parse_node(prev.as_token().unwrap().green().text());
                                    node = node
                                        .insert_child(child.index() + 1, whitespace.green().into());
                                }
                                return Ok(Some(parse_node(&node.to_string())));
                            }
                        }
                    }
                } else {
                    return Err(WalkerError::UnexpectedNodeKind {
                        expected: SyntaxKind::NODE_ATTRPATH_VALUE,
                        found: toplevel.kind(),
                    });
                }
            }
        }
        Ok(None)
    }
    fn walk_inputs(
        &mut self,
        node: SyntaxNode,
        ctx: &Option<Context>,
        change: &Change,
    ) -> Option<SyntaxNode> {
        tracing::debug!("WalkInputs: \n{node}\n with ctx: {ctx:?}");
        tracing::debug!("WalkInputsKind: {:?}", node.kind());
        // Handle special node types at the top level
        match node.kind() {
            SyntaxKind::NODE_ATTRPATH => {
                if let Some(result) = self.handle_attrpath_follows(&node, change) {
                    return Some(result);
                }
            }
            SyntaxKind::NODE_ATTR_SET
            | SyntaxKind::NODE_ATTRPATH_VALUE
            | SyntaxKind::NODE_IDENT => {}
            _ => {}
        }
        for child in node.children_with_tokens() {
            tracing::debug!("Inputs Child Kind: {:?}", child.kind());
            tracing::debug!("Inputs Child: {child}");
            tracing::debug!("Inputs Child Len: {}", child.to_string().len());
            match child.kind() {
                SyntaxKind::NODE_ATTRPATH_VALUE => {
                    if let Some(result) =
                        self.handle_child_attrpath_value(&node, &child, ctx, change)
                    {
                        return Some(result);
                    }
                }
                SyntaxKind::NODE_IDENT => {
                    if let Some(result) = self.handle_child_ident(&child, ctx, change) {
                        return Some(result);
                    }
                }
                _ => {
                    tracing::debug!("UNMATCHED KIND: {:?}", child.kind());
                    tracing::debug!("UNMATCHED PATH: {}", child);
                }
            }
        }
        None
    }

    /// Handle flat-style URL attribute: `inputs.foo.url = "..."`
    /// Returns Some(node) if a modification was made.
    fn handle_flat_url(
        &mut self,
        input_id: &SyntaxNode,
        url: &SyntaxNode,
        ctx: &Option<Context>,
        change: &Change,
    ) -> Option<SyntaxNode> {
        let id_str = input_id.to_string();
        tracing::debug!("This is an url from {} - {}", input_id, url);
        let input = Input::with_url(id_str.clone(), url.to_string(), url.text_range());
        self.insert_with_ctx(id_str.clone(), input, ctx);

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
            tracing::debug!("Changing URL for {change_id} to {new_uri} (toplevel flat style)");
            return Some(make_quoted_string(new_uri));
        }

        None
    }

    /// Handle flat-style flake attribute: `inputs.foo.flake = false`
    /// Returns Some(node) if a modification was made.
    fn handle_flat_flake(
        &mut self,
        input_id: &SyntaxNode,
        is_flake: &SyntaxNode,
        ctx: &Option<Context>,
        change: &Change,
    ) -> Option<SyntaxNode> {
        let id_str = input_id.to_string();
        tracing::debug!("This id {} is a flake: {}", input_id, is_flake);

        if should_remove_input(change, ctx, &id_str) {
            return Some(empty_node());
        }

        None
    }

    /// Handle nested input attributes like `inputs.foo = { url = "..."; ... }`
    /// Returns Some(node) if a modification was made.
    fn handle_nested_input(
        &mut self,
        input_id: &SyntaxNode,
        nested_attr: &SyntaxNode,
        ctx: &Option<Context>,
        change: &Change,
    ) -> Option<SyntaxNode> {
        let id_str = input_id.to_string();
        tracing::debug!("Nested input: {}", nested_attr);

        for attr in nested_attr.children() {
            tracing::debug!("Nested input attr: {}, from: {}", attr, input_id);

            for binding in attr.children() {
                if binding.to_string() == "url" {
                    let url = binding.next_sibling().unwrap();
                    tracing::debug!("This is an url: {} - {}", input_id, url);
                    let input =
                        Input::with_url(id_str.clone(), url.to_string(), input_id.text_range());
                    self.insert_with_ctx(id_str.clone(), input, ctx);
                }
                if should_remove_input(change, ctx, &id_str) {
                    return Some(empty_node());
                }
                tracing::debug!("Nested input attr binding: {}", binding);
            }

            let context = id_str.clone().into();
            tracing::debug!("Walking inputs with: {attr}, context: {context:?}");
            if let Some(result) = self.walk_input(&attr, &Some(context), change) {
                tracing::debug!("Adjusted change: {result}");
                tracing::debug!(
                    "Adjusted change is_empty: {}",
                    result.to_string().is_empty()
                );
                // TODO: adjust node correctly if the change is not empty
                let replacement =
                    Self::remove_child_with_whitespace(nested_attr, &attr, attr.index());
                tracing::debug!("Replacement: {}", replacement);
                return Some(replacement);
            }
        }

        None
    }

    /// Handle a NODE_IDENT child node during input walking.
    /// Processes flat-style input declarations like `inputs.nixpkgs.url = "..."`
    /// Returns Some(node) if a modification was made, None otherwise.
    fn handle_child_ident(
        &mut self,
        child: &rnix::SyntaxElement,
        ctx: &Option<Context>,
        change: &Change,
    ) -> Option<SyntaxNode> {
        tracing::debug!("Node PATH: {}", child);

        // Handle "inputs" identifier with next sibling
        if child.to_string() == "inputs"
            && let Some(next_sibling) = child.as_node().unwrap().next_sibling()
        {
            match next_sibling.kind() {
                SyntaxKind::NODE_IDENT => {
                    tracing::debug!("NODE_IDENT input: {}", next_sibling);
                    if let Some(url_id) = next_sibling.next_sibling() {
                        if url_id.kind() == SyntaxKind::NODE_IDENT {
                            if let Some(value) =
                                child.as_node().unwrap().parent().unwrap().next_sibling()
                            {
                                if url_id.to_string() == "url" {
                                    if let Some(result) =
                                        self.handle_flat_url(&next_sibling, &value, ctx, change)
                                    {
                                        return Some(result);
                                    }
                                } else if url_id.to_string() == "flake" {
                                    if let Some(result) =
                                        self.handle_flat_flake(&next_sibling, &value, ctx, change)
                                    {
                                        return Some(result);
                                    }
                                } else {
                                    tracing::debug!("Unhandled input: {}", next_sibling);
                                }
                            }
                        } else {
                            tracing::debug!("Unhandled input: {}", next_sibling);
                        }
                    } else if let Some(nested_attr) =
                        child.as_node().unwrap().parent().unwrap().next_sibling()
                        && let Some(result) =
                            self.handle_nested_input(&next_sibling, &nested_attr, ctx, change)
                    {
                        return Some(result);
                    }
                }
                SyntaxKind::NODE_ATTR_SET => {}
                _ => {
                    tracing::debug!("Unhandled input kind: {:?}", next_sibling.kind());
                    tracing::debug!("Unhandled input: {}", next_sibling);
                }
            }
        }

        // Handle flat tree attributes like "inputs.X.Y"
        if child.to_string().starts_with("inputs") {
            let child_node = child.as_node().unwrap();
            let id = child_node.next_sibling().unwrap();
            let context = id.to_string().into();
            tracing::debug!("Walking inputs with: {child}, context: {context:?}");
            if let Some(_replacement) = self.walk_inputs(child_node.clone(), &Some(context), change)
            {
                // TODO: Handle flat tree attribute replacement
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
        &mut self,
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
        if let Some(replacement) = self.walk_input(child_node, &ctx, change) {
            tracing::debug!("Child Id: {}", child.index());
            tracing::debug!("Input replacement node: {}", parent);
            let mut green = parent
                .green()
                .replace_child(child.index(), replacement.green().into());

            // Remove adjacent whitespace if the replacement is empty
            if replacement.text().is_empty() {
                green = strip_whitespace_after_child(green, child);
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

            tracing::debug!("green: {}", green);
            tracing::debug!("node: {}", parent);
            tracing::debug!("node kind: {:?}", parent.kind());

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
        &mut self,
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
        self.insert_with_ctx(follows_id.to_string(), input, &ctx);

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
        &mut self,
        node: &SyntaxNode,
        child: &SyntaxNode,
        attr: &SyntaxNode,
        ctx: &Option<Context>,
        change: &Change,
    ) -> Option<SyntaxNode> {
        if let Some(prev_id) = attr.prev_sibling() {
            if let Change::Remove { id } = change
                && id.to_string() == prev_id.to_string()
            {
                tracing::debug!("Removing: {id}");
                return Some(empty_node());
            }
            if let Change::Change { id, uri, .. } = change
                && let Some(id) = id
                && *id == prev_id.to_string()
                && let Some(uri) = uri
            {
                tracing::debug!("Changing URL for {id} to {uri}");
                if let Some(url_node) = child.next_sibling() {
                    let new_url = make_quoted_string(uri);
                    let green = node
                        .green()
                        .replace_child(url_node.index(), new_url.green().into());
                    return Some(parse_node(&green.to_string()));
                }
            }
            if let Some(sibling) = child.next_sibling() {
                tracing::debug!("This is an url from {} - {}", prev_id, sibling);
                let input = Input::with_url(
                    prev_id.to_string(),
                    sibling.to_string(),
                    sibling.text_range(),
                );
                self.insert_with_ctx(prev_id.to_string(), input, ctx);
            }
        }

        tracing::debug!("This is the parent: {}", child.parent().unwrap());
        tracing::debug!(
            "This is the next_sibling: {}",
            child.next_sibling().unwrap()
        );

        // Handle nested follows within url attribute
        if let Some(parent) = child.parent()
            && let Some(sibling) = parent.next_sibling()
        {
            // TODO: this is only matched, when url is the first child
            // TODO: Is this correct?
            tracing::debug!("This is a possible follows attribute:{} {}", attr, sibling);
            if let Some(nested_child) = sibling.first_child()
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
                        tracing::debug!(
                            "The following attribute follows: {id}:{follows} is nested inside the attr: {ctx:?}"
                        );
                        let input = Input::with_url(
                            id.to_string(),
                            follows.to_string(),
                            follows.text_range(),
                        );
                        self.insert_with_ctx(id.to_string(), input, ctx);
                        if change.is_remove()
                            && let Some(id) = change.id()
                            && id.matches_with_ctx(&follows.to_string(), ctx.clone())
                        {
                            return Some(empty_node());
                        }
                    }
                }
            }
        }
        None
    }

    /// Handle "flake" attribute within an input's ATTRPATH.
    fn handle_flake_attr(
        &mut self,
        attr: &SyntaxNode,
        ctx: &Option<Context>,
        change: &Change,
    ) -> Option<SyntaxNode> {
        if let Some(input_id) = attr.prev_sibling() {
            if let Some(is_flake) = attr.parent().unwrap().next_sibling() {
                tracing::debug!(
                    "The following attribute is a flake: {input_id}:{is_flake} is nested inside the context: {ctx:?}"
                );
                let mut input = Input::new(input_id.to_string());
                input.flake = is_flake.to_string().parse().unwrap();
                let text_range = input_id.text_range();
                input.range = crate::input::Range::from_text_range(text_range);
                self.insert_with_ctx(input_id.to_string(), input, ctx);
                if change.is_remove()
                    && let Some(id) = change.id()
                    && id.matches_with_ctx(&input_id.to_string(), ctx.clone())
                {
                    return Some(empty_node());
                }
            }
        } else {
            // TODO: handle this.
            // This happens, when there is a nested node.
            tracing::info!("Nested: This is not handled yet.");
        }
        None
    }

    /// Handle "follows" attribute within an input's ATTRPATH.
    fn handle_follows_attr(
        &mut self,
        attr: &SyntaxNode,
        ctx: &Option<Context>,
        change: &Change,
    ) -> Option<SyntaxNode> {
        // Construct the follows attribute
        // TODO:
        // - check for possible removal / change
        let id = attr.prev_sibling().unwrap();
        let follows = attr.parent().unwrap().next_sibling().unwrap();
        tracing::debug!(
            "The following attribute follows: {id}:{follows} is nested inside the attr: {ctx:?}"
        );
        // TODO: Construct follows attribute if not yet ready.
        // For now assume that the url is the first attribute.
        // This assumption doesn't generally hold true.
        let input = Input::with_url(id.to_string(), follows.to_string(), follows.text_range());
        self.insert_with_ctx(id.to_string(), input.clone(), ctx);
        if let Some(id) = change.id()
            && let Some(ctx) = ctx
            && id.matches_with_ctx(input.id(), Some(ctx.clone()))
            && change.is_remove()
        {
            return Some(empty_node());
        }
        None
    }

    /// Handle NODE_ATTRPATH within an input node.
    /// Dispatches to url, flake, and follows handlers.
    fn handle_input_attrpath(
        &mut self,
        node: &SyntaxNode,
        child: &SyntaxNode,
        ctx: &Option<Context>,
        change: &Change,
    ) -> Option<SyntaxNode> {
        for attr in child.children() {
            tracing::debug!("Child of ATTRPATH: {}", child);
            tracing::debug!("Child of ATTR: {}", attr);

            let attr_name = attr.to_string();
            match attr_name.as_str() {
                "url" => {
                    if let Some(result) = self.handle_url_attr(node, child, &attr, ctx, change) {
                        return Some(result);
                    }
                }
                "flake" => {
                    if let Some(result) = self.handle_flake_attr(&attr, ctx, change) {
                        return Some(result);
                    }
                }
                "follows" => {
                    if let Some(result) = self.handle_follows_attr(&attr, ctx, change) {
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
        &mut self,
        node: &SyntaxNode,
        child: &SyntaxNode,
        ctx: &Option<Context>,
        change: &Change,
    ) -> Option<SyntaxNode> {
        for attr in child.children() {
            tracing::debug!("Child of ATTRSET KIND: {:?}", attr.kind());
            tracing::debug!("Child of ATTRSET: {}", attr);
            for leaf in attr.children() {
                tracing::debug!("LEAF of ATTRSET KIND: {:?}", leaf.kind());
                tracing::debug!("LEAF of ATTRSET: {}", leaf);

                if leaf.to_string() == "url" {
                    let id = child.prev_sibling().unwrap();
                    let uri = leaf.next_sibling().unwrap();
                    tracing::debug!("This is an url from {} - {}", id, uri);
                    let input = Input::with_url(id.to_string(), uri.to_string(), uri.text_range());
                    self.insert_with_ctx(id.to_string(), input, ctx);

                    // Remove matched node.
                    if let Change::Remove { id: candidate } = change
                        && candidate.to_string() == id.to_string()
                    {
                        tracing::debug!("Removing: {id}");
                        return Some(empty_node());
                    }

                    if let Change::Change {
                        id: Some(change_id),
                        uri: Some(new_uri),
                        ..
                    } = change
                        && *change_id == id.to_string()
                    {
                        tracing::debug!("Changing URL for {change_id} to {new_uri} (nested style)");
                        let new_url = make_quoted_string(new_uri);
                        let green = attr.green().replace_child(
                            leaf.next_sibling().unwrap().index(),
                            new_url.green().into(),
                        );
                        let new_attr = parse_node(&green.to_string());
                        let green = child
                            .green()
                            .replace_child(attr.index(), new_attr.green().into());
                        let new_child = parse_node(&green.to_string());
                        let green = node
                            .green()
                            .replace_child(child.index(), new_child.green().into());
                        return Some(parse_node(&green.to_string()));
                    }
                }

                if leaf.to_string().starts_with("inputs") {
                    let id = child.prev_sibling().unwrap();
                    let context = id.to_string().into();
                    tracing::debug!("Walking inputs with: {attr}, context: {context:?}");
                    if let Some(replacement) =
                        self.walk_inputs(child.clone(), &Some(context), change)
                    {
                        // TODO: adjustment of whitespace, if node is empty
                        // TODO: if it leaves an empty attr, then remove whole?
                        let tree = node
                            .green()
                            .replace_child(child.index(), replacement.green().into());
                        return Some(parse_node(&tree.to_string()));
                    }
                    tracing::debug!("Child of ATTRSET KIND: {:?}", leaf.kind());
                    tracing::debug!("Child of ATTRSET CHILD: {}", leaf);
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
    fn walk_input(
        &mut self,
        node: &SyntaxNode,
        ctx: &Option<Context>,
        change: &Change,
    ) -> Option<SyntaxNode> {
        tracing::debug!("\nInput: {node}\n with ctx: {ctx:?}");
        for child in node.children() {
            tracing::debug!("Kind: {:?}", child.kind());
            tracing::debug!("Kind: {}", child);

            if child.kind() == SyntaxKind::NODE_ATTRPATH
                && let Some(result) = self.handle_input_attrpath(node, &child, ctx, change)
            {
                return Some(result);
            }

            if child.kind() == SyntaxKind::NODE_ATTR_SET
                && let Some(result) = self.handle_input_attr_set(node, &child, ctx, change)
            {
                return Some(result);
            }

            tracing::debug!("Child: {}", child);
        }
        None
    }
}
