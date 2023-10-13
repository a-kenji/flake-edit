use std::collections::HashMap;

use rnix::{Root, SyntaxKind, SyntaxNode};

use crate::{change::Change, input::Input};

// TODO:
// - parse out inputs
// - SyntaxKind(44) [inputs]
// - parse follows attribute and attrset outof the -> SyntaxKind(76) [attrset]
//
// // TODO: hopefully we won't need these codes where we are going
// NODE_STRING 63,
// NODE_IDENT 58,
// TOKEN_IDENT 44,
// TOKEN_DOT 21,
// NODE_ROOT 75,
// NODE_ATTR_SET 76,
// NODE_ATTRPATH 55,
// TOKEN_URI 49,

#[derive(Debug, Clone)]
pub struct Walker {
    pub root: SyntaxNode,
    pub inputs: HashMap<String, Input>,
    pub add_toplevel: bool,
}

#[derive(Debug, Clone)]
/// A helper for the [`Walker`], in order to hold context while traversing the tree.
struct Context {
    level: Vec<String>,
}

impl Context {
    fn new(level: Vec<String>) -> Self {
        Self { level }
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
    pub fn walk(&mut self, change: &Change) -> Option<SyntaxNode> {
        let cst = &self.root;
        if cst.kind() != SyntaxKind::NODE_ROOT {
            // TODO: handle this as an error
            panic!("Should be a topevel node.")
        } else {
            self.walk_toplevel(cst.clone(), None, change)
        }
    }
    /// Insert a new Input node at the correct position
    /// or update it with new information.
    fn insert_with_ctx(&mut self, id: String, input: Input, ctx: &Option<Context>) {
        tracing::debug!("Inserting id: {id}, input: {input:?} with: ctx: {ctx:?}");

        if let Some(ctx) = ctx {
            // TODO: add more nesting
            if let Some(follows) = ctx.level.first() {
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
                node.url = input.url;
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
        let mut green = parent.green().remove_child(index);
        if let Some(prev) = node.prev_sibling_or_token() {
            if let SyntaxKind::TOKEN_WHITESPACE = prev.kind() {
                green = green.remove_child(prev.index());
            }
        } else if let Some(next) = node.next_sibling_or_token() {
            if let SyntaxKind::TOKEN_WHITESPACE = next.kind() {
                green = green.remove_child(next.index());
            }
        }
        Root::parse(green.to_string().as_str()).syntax()
    }
    /// Traverse the toplevel `flake.nix` file.
    /// It should consist of three attribute keys:
    /// - description
    /// - inputs
    /// - outputs
    fn walk_toplevel(
        &mut self,
        node: SyntaxNode,
        ctx: Option<Context>,
        change: &Change,
    ) -> Option<SyntaxNode> {
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
                            break;
                        }
                        if child.to_string() == "inputs" {
                            if let Some(replacement) =
                                self.walk_inputs(child.next_sibling().unwrap(), &ctx, &change)
                            {
                                tracing::debug!("Replacement Noode: {replacement}");
                                let green = toplevel.green().replace_child(
                                    child.next_sibling().unwrap().index(),
                                    replacement.green().into(),
                                );
                                let green = toplevel.replace_with(green);
                                let node = Root::parse(green.to_string().as_str()).syntax();
                                return Some(node);
                            }
                        } else if child.to_string().starts_with("inputs") {
                            // This is a toplevel node, of the form:
                            // input.id ...
                            // If the node should be empty,
                            // it's toplevel should be empty too.
                            if let Some(replacement) = self.walk_inputs(child.clone(), &ctx, change)
                            {
                                if replacement.to_string().is_empty() {
                                    // let green =
                                    //     toplevel.parent().unwrap().green().replace_child(
                                    //         child.index(),
                                    //         replacement.green().into(),
                                    //     );
                                    // let green = toplevel.replace_with(green);
                                    // let node = Root::parse(green.to_string().as_str()).syntax();
                                    // let green =
                                    //     toplevel.replace_with(replacement.green().into());
                                    // let mut green = root.green().remove_child(toplevel.index());
                                    let node = Self::remove_child_with_whitespace(
                                        &root,
                                        &toplevel,
                                        toplevel.index(),
                                    );
                                    return Some(node);
                                }
                            }
                        };
                        if child.to_string() == "outputs" && self.add_toplevel {
                            if let Change::Add { id, uri } = change {
                                let addition = Root::parse(&format!(
                                    "inputs.{} = \"{}\";",
                                    id.clone().unwrap(),
                                    uri.clone().unwrap()
                                ))
                                .syntax();
                                // TODO Guard against indices that would be out of range here.
                                if toplevel.index() > 0 {
                                    let mut node = root.green().insert_child(
                                        toplevel.index() - 1,
                                        addition.green().into(),
                                    );
                                    root.children()
                                        .find(|c| c.index() == toplevel.index() - 2)
                                        .map(|c| {
                                            if let Some(prev) = c.prev_sibling_or_token() {
                                                if prev.kind() == SyntaxKind::TOKEN_WHITESPACE {
                                                    let whitespace = Root::parse(&format!(
                                                        "{}",
                                                        prev.as_token().unwrap().green().text()
                                                    ))
                                                    .syntax();
                                                    node = node.insert_child(
                                                        toplevel.index() - 1,
                                                        whitespace.green().into(),
                                                    );
                                                }
                                            } else if let Some(prev) = c.next_sibling_or_token() {
                                                if prev.kind() == SyntaxKind::TOKEN_WHITESPACE {
                                                    let whitespace = Root::parse(&format!(
                                                        "{}",
                                                        prev.as_token().unwrap().green().text()
                                                    ))
                                                    .syntax();
                                                    node = node.insert_child(
                                                        toplevel.index() - 1,
                                                        whitespace.green().into(),
                                                    );
                                                }
                                            }
                                        });
                                    //     child.prev_sibling_or_token() {
                                    //     if prev.kind() == SyntaxKind::TOKEN_WHITESPACE {
                                    //         let whitespace =
                                    //             Root::parse(prev.as_token().unwrap().green().text())
                                    //                 .syntax();
                                    //         node = node.insert_child(
                                    //             child.index() + 1,
                                    //             whitespace.green().into(),
                                    //         );
                                    //     }
                                    // }
                                    if let Some(prev) = child.next_sibling_or_token() {
                                        if prev.kind() == SyntaxKind::TOKEN_WHITESPACE {
                                            let whitespace = Root::parse(
                                                prev.as_token().unwrap().green().text(),
                                            )
                                            .syntax();
                                            node = node.insert_child(
                                                child.index() + 1,
                                                whitespace.green().into(),
                                            );
                                        }
                                    }
                                    return Some(Root::parse(&node.to_string()).syntax());
                                }
                            }
                        }
                    }
                } else {
                    // TODO: handle
                    panic!("Should be a NODE_ATTRPATH_VALUE");
                }
            }
        }
        None
    }
    fn walk_inputs(
        &mut self,
        node: SyntaxNode,
        ctx: &Option<Context>,
        change: &Change,
    ) -> Option<SyntaxNode> {
        tracing::debug!("WalkInputs: \n{node}\n with ctx: {ctx:?}");
        tracing::debug!("WalkInputsKind: {:?}", node.kind());
        match node.kind() {
            SyntaxKind::NODE_ATTR_SET => {}
            SyntaxKind::NODE_ATTRPATH_VALUE => {}
            SyntaxKind::NODE_IDENT => {}
            SyntaxKind::NODE_ATTRPATH => {
                let maybe_follows_id = node
                    .children()
                    .find(|child| child.to_string() == "follows")
                    .and_then(|input_child| input_child.prev_sibling());
                if let Some(follows_id) = maybe_follows_id {
                    let maybe_input_id = node
                        .children()
                        .find(|child| child.to_string() == "inputs")
                        .and_then(|input_child| input_child.next_sibling());
                    let ctx = maybe_input_id.map(|id| Context::new(vec![id.to_string()]));
                    let mut input = Input::new(follows_id.to_string());
                    input.url = node.next_sibling().unwrap().to_string();
                    self.insert_with_ctx(follows_id.to_string(), input, &ctx);
                }
            }
            _ => {}
        }
        for child in node.children_with_tokens() {
            tracing::debug!("Inputs Child Kind: {:?}", child.kind());
            tracing::debug!("Inputs Child: {child}");
            tracing::debug!("Inputs Child Len: {}", child.to_string().len());
            match child.kind() {
                SyntaxKind::NODE_ATTRPATH_VALUE => {
                    // TODO: Append to context, instead of creating a new one.
                    let ctx = if ctx.is_none() {
                        let maybe_input_id = child.as_node().unwrap().children().find_map(|c| {
                            c.children()
                                .find(|child| child.to_string() == "inputs")
                                .and_then(|input_child| input_child.prev_sibling())
                        });
                        maybe_input_id.map(|id| Context::new(vec![id.to_string()]))
                    } else {
                        ctx.clone()
                    };
                    if let Some(replacement) =
                        self.walk_input(child.as_node().unwrap(), &ctx, change)
                    {
                        tracing::debug!("Child Id: {}", child.index());
                        tracing::debug!("Input replacement node: {}", node);
                        let mut green = node
                            .green()
                            .replace_child(child.index(), replacement.green().into());
                        if replacement.text().is_empty() {
                            let prev = child.prev_sibling_or_token();
                            if let Some(prev) = prev {
                                if let SyntaxKind::TOKEN_WHITESPACE = prev.kind() {
                                    green = green.remove_child(prev.index());
                                }
                            } else if let Some(next) = child.next_sibling_or_token() {
                                if let SyntaxKind::TOKEN_WHITESPACE = next.kind() {
                                    green = green.remove_child(next.index());
                                }
                            }
                        }
                        let node = Root::parse(green.to_string().as_str()).syntax();
                        return Some(node);
                    } else if change.is_some() && change.id().is_some() {
                        if let Change::Add { id, uri } = change {
                            let uri = Root::parse(&format!(
                                "{}.url = \"{}\";",
                                id.clone().unwrap(),
                                uri.clone().unwrap(),
                            ))
                            .syntax();
                            let mut green =
                                node.green().insert_child(child.index(), uri.green().into());
                            let prev = child.prev_sibling_or_token().unwrap();
                            tracing::debug!("Token:{}", prev);
                            tracing::debug!("Token Kind: {:?}", prev.kind());
                            if prev.kind() == SyntaxKind::TOKEN_WHITESPACE {
                                let whitespace =
                                    Root::parse(prev.as_token().unwrap().green().text()).syntax();
                                green = green
                                    .insert_child(child.index() + 1, whitespace.green().into());
                            }
                            tracing::debug!("green: {}", green);
                            tracing::debug!("node: {}", node);
                            tracing::debug!("node kind: {:?}", node.kind());
                            let node = Root::parse(green.to_string().as_str()).syntax();
                            return Some(node);
                        }
                    }
                }
                SyntaxKind::NODE_IDENT => {
                    tracing::debug!("Node PATH: {}", child);
                    if child.to_string() == "inputs" {
                        if let Some(next_sibling) = child.as_node().unwrap().next_sibling() {
                            match next_sibling.kind() {
                                SyntaxKind::NODE_IDENT => {
                                    tracing::debug!("NODE_IDENT input: {}", next_sibling);
                                    tracing::debug!("NODE_IDENT input: {}", next_sibling);
                                    if let Some(url_id) = next_sibling.next_sibling() {
                                        match url_id.kind() {
                                            SyntaxKind::NODE_IDENT => {
                                                if url_id.to_string() == "url" {
                                                    if let Some(url) = child
                                                        .as_node()
                                                        .unwrap()
                                                        .parent()
                                                        .unwrap()
                                                        .next_sibling()
                                                    {
                                                        tracing::debug!(
                                                            "This is an url from {} - {}",
                                                            next_sibling,
                                                            url
                                                        );
                                                        let mut input =
                                                            Input::new(next_sibling.to_string());
                                                        input.url = url.to_string();
                                                        self.insert_with_ctx(
                                                            next_sibling.to_string(),
                                                            input,
                                                            ctx,
                                                        );
                                                        if change.is_some() && change.is_remove() {
                                                            if let Some(id) = change.id() {
                                                                if id == next_sibling.to_string() {
                                                                    let replacement =
                                                                        Root::parse("").syntax();
                                                                    // let green = node
                                                                    //     .green()
                                                                    //     .replace_child(
                                                                    //         child.index(),
                                                                    //         replacement
                                                                    //             .green()
                                                                    //             .into(),
                                                                    //     );
                                                                    // let green = toplevel
                                                                    //     .replace_with(green);
                                                                    // let node = Root::parse(
                                                                    //     green
                                                                    //         .to_string()
                                                                    //         .as_str(),
                                                                    // )
                                                                    // .syntax();
                                                                    tracing::debug!(
                                                                        "Noode: {node}"
                                                                    );
                                                                    return Some(replacement);
                                                                }
                                                            }
                                                            if let Some(ctx) = ctx {
                                                                if *ctx.level.first().unwrap()
                                                                    == next_sibling.to_string()
                                                                {
                                                                    let replacement =
                                                                        Root::parse("").syntax();
                                                                    return Some(replacement);
                                                                }
                                                            }
                                                        }
                                                    }
                                                } else {
                                                    tracing::debug!(
                                                        "Unhandled input: {}",
                                                        next_sibling
                                                    );
                                                }
                                            }
                                            _ => {
                                                tracing::debug!(
                                                    "Unhandled input: {}",
                                                    next_sibling
                                                );
                                            }
                                        }
                                    } else {
                                        tracing::debug!("Unhandled input: {}", next_sibling);
                                        if let Some(nested_attr) = child
                                            .as_node()
                                            .unwrap()
                                            .parent()
                                            .unwrap()
                                            .next_sibling()
                                        {
                                            tracing::debug!("Nested input: {}", nested_attr);
                                            for attr in nested_attr.children() {
                                                tracing::debug!(
                                                    "Nested input attr: {}, from: {}",
                                                    attr,
                                                    next_sibling
                                                );

                                                for binding in attr.children() {
                                                    if binding.to_string() == "url" {
                                                        let url = binding.next_sibling().unwrap();
                                                        tracing::debug!(
                                                            "This is an url: {} - {}",
                                                            next_sibling,
                                                            url
                                                        );
                                                        let mut input =
                                                            Input::new(next_sibling.to_string());
                                                        input.url = url.to_string();
                                                        self.insert_with_ctx(
                                                            next_sibling.to_string(),
                                                            input,
                                                            ctx,
                                                        );
                                                    }
                                                    if change.is_some() && change.is_remove() {
                                                        if let Some(id) = change.id() {
                                                            if id == next_sibling.to_string() {
                                                                let replacement =
                                                                    Root::parse("").syntax();
                                                                tracing::debug!("Noode: {node}");
                                                                return Some(replacement);
                                                            }
                                                        }
                                                    }
                                                    tracing::debug!(
                                                        "Nested input attr binding: {}",
                                                        binding
                                                    );
                                                }
                                                let context =
                                                    Context::new(vec![next_sibling.to_string()]);
                                                tracing::debug!("Walking inputs with: {attr}, context: {context:?}");
                                                if let Some(change) =
                                                    self.walk_input(&attr, &Some(context), change)
                                                {
                                                    println!("Nested change: {change}");
                                                    panic!("Matched nested");
                                                }
                                            }
                                        }
                                    }
                                }
                                SyntaxKind::NODE_ATTR_SET => {}
                                _ => {
                                    tracing::debug!(
                                        "Unhandled input kind: {:?}",
                                        next_sibling.kind()
                                    );
                                    tracing::debug!("Unhandled input: {}", next_sibling);
                                }
                            }
                        }
                    }
                    // TODO: flat tree attributes
                    if child.to_string().starts_with("inputs") {
                        let child_node = child.as_node().unwrap();
                        let id = child_node.next_sibling().unwrap();
                        let context = Context::new(vec![id.to_string()]);
                        tracing::debug!("Walking inputs with: {child}, context: {context:?}");
                        if let Some(_replacement) =
                            self.walk_inputs(child_node.clone(), &Some(context), change)
                        {
                            panic!("Not yet implemented");
                        }
                    }
                    if let Some(parent) = child.parent() {
                        tracing::debug!("Children Parent Child: {}", child);
                        tracing::debug!("Children Parent Child Kind: {:?}", child.kind());
                        tracing::debug!("Children Parent Kind: {:?}", parent.kind());
                        tracing::debug!("Children Parent: {}", parent);
                        tracing::debug!("Children Parent Context: {:?}", ctx);
                        if let Some(sibling) = parent.next_sibling() {
                            tracing::debug!("Children Sibling: {}", sibling);
                        }
                        for child in parent.children() {
                            tracing::debug!("Children Sibling --: {}", child);
                        }
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
        for (i, child) in node.children().enumerate() {
            tracing::debug!("Kind #:{i} {:?}", child.kind());
            tracing::debug!("Kind #:{i} {}", child);
            if child.kind() == SyntaxKind::NODE_ATTRPATH {
                for attr in child.children() {
                    tracing::debug!("Child of ATTRPATH #:{i} {}", child);
                    tracing::debug!("Child of ATTR #:{i} {}", attr);
                    if attr.to_string() == "url" {
                        if let Some(prev_id) = attr.prev_sibling() {
                            if let Change::Remove { id } = change {
                                if *id == prev_id.to_string() {
                                    tracing::debug!("Removing: {id}");
                                    let empty = Root::parse("").syntax();
                                    return Some(empty);
                                }
                            }
                            if let Some(sibling) = child.next_sibling() {
                                tracing::debug!("This is an url from {} - {}", prev_id, sibling);
                                let mut input = Input::new(prev_id.to_string());
                                input.url = sibling.to_string();
                                self.insert_with_ctx(prev_id.to_string(), input, ctx);
                            }
                        }
                        tracing::debug!("This is the parent: {}", child.parent().unwrap());
                        tracing::debug!(
                            "This is the next_sibling: {}",
                            child.next_sibling().unwrap()
                        );
                        if let Some(parent) = child.parent() {
                            if let Some(sibling) = parent.next_sibling() {
                                //TODO: this is only matched, when url is the first child
                                // TODO: Is this correct?
                                tracing::debug!(
                                    "This is a possible follows attribute:{} {}",
                                    attr,
                                    sibling
                                );
                                if let Some(child) = sibling.first_child() {
                                    if child.to_string() == "inputs" {
                                        if let Some(attr_set) = child.next_sibling() {
                                            if SyntaxKind::NODE_ATTR_SET == attr_set.kind() {
                                                for attr in attr_set.children() {
                                                    let is_follows = attr
                                                        .first_child()
                                                        .unwrap()
                                                        .first_child()
                                                        .unwrap()
                                                        .next_sibling()
                                                        .unwrap();

                                                    if is_follows.to_string() == "follows" {
                                                        let id = is_follows.prev_sibling().unwrap();
                                                        let follows = attr
                                                            .first_child()
                                                            .unwrap()
                                                            .next_sibling()
                                                            .unwrap();
                                                        tracing::debug!("The following attribute follows: {id}:{follows} is nested inside the attr: {ctx:?}");
                                                        let mut input = Input::new(id.to_string());
                                                        input.url = follows.to_string();
                                                        self.insert_with_ctx(
                                                            id.to_string(),
                                                            input,
                                                            ctx,
                                                        );
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if attr.to_string() == "follows" {
                        // Construct the follows attribute
                        // TODO:
                        // - check for possible removal / change
                        let id = attr.prev_sibling().unwrap();
                        let follows = attr.parent().unwrap().next_sibling().unwrap();
                        tracing::debug!("The following attribute follows: {id}:{follows} is nested inside the attr: {ctx:?}");
                        // TODO: Construct follows attribute if not yet ready.
                        // For now assume that the url is the first attribute.
                        // This assumption doesn't generally hold true.
                        let mut input = Input::new(id.to_string());
                        input.url = follows.to_string();
                        self.insert_with_ctx(id.to_string(), input, ctx);
                        if let Some(id) = change.id() {
                            if let Some(ctx) = ctx {
                                if id == *ctx.level.first().unwrap() && change.is_remove() {
                                    let replacement = Root::parse("").syntax();
                                    return Some(replacement);
                                }
                            }
                        }
                    }
                }
            }
            if child.kind() == SyntaxKind::NODE_ATTR_SET {
                for attr in child.children() {
                    tracing::debug!("Child of ATTRSET KIND #:{i} {:?}", attr.kind());
                    tracing::debug!("Child of ATTRSET #:{i} {}", attr);
                    for leaf in attr.children() {
                        tracing::debug!("LEAF of ATTRSET KIND #:{i} {:?}", leaf.kind());
                        tracing::debug!("LEAF of ATTRSET #:{i} {}", leaf);
                        if leaf.to_string() == "url" {
                            let id = child.prev_sibling().unwrap();
                            let uri = leaf.next_sibling().unwrap();
                            tracing::debug!("This is an url from {} - {}", id, uri,);
                            let mut input = Input::new(id.to_string());
                            input.url = uri.to_string();
                            self.insert_with_ctx(id.to_string(), input, ctx);

                            // Remove matched node.
                            if let Change::Remove { id: candidate } = change {
                                if *candidate == id.to_string() {
                                    tracing::debug!("Removing: {id}");
                                    let empty = Root::parse("").syntax();
                                    return Some(empty);
                                }
                            }
                        }
                        if leaf.to_string().starts_with("inputs") {
                            let id = child.prev_sibling().unwrap();
                            let context = Context::new(vec![id.to_string()]);
                            tracing::debug!("Walking inputs with: {attr}, context: {context:?}");
                            // panic!("Walking inputs with: {attr}, context: {context:?}");
                            if let Some(replacement) =
                                self.walk_inputs(child.clone(), &Some(context), change)
                            // self.walk_inputs(attr.clone(), &Some(context))
                            {
                                // if let Some(change) = self.walk_input(&attr, &Some(context)) {
                                //     println!("Nested change: {change}");
                                panic!("Matched nested");
                                // }
                            }
                            tracing::debug!("Child of ATTRSET KIND #:{i} {:?}", leaf.kind());
                            tracing::debug!("Child of ATTRSET CHILD #:{i} {}", leaf);
                        }
                    }
                }
            }
            tracing::debug!("Child #:{i} {}", child);
        }
        None
    }
}

// #[cfg(test)]
// mod tests {
//     use super::*;
//     fn minimal_flake() -> &'static str {
//         r#"
//         {
//   description = "flk - a tui for your flakes.";
//
//   inputs.nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
//
//   inputs.rust-overlay = {
//     url = "github:oxalica/rust-overlay";
//     inputs.nixpkgs.follows = "nixpkgs";
//     inputs.flake-utils.follows = "flake-utils";
//   }
//
//   inputs.crane = {
//     url = "github:ipetkov/crane";
//     inputs.nixpkgs.follows = "nixpkgs";
//     inputs.rust-overlay.follows = "rust-overlay";
//     inputs.flake-utils.follows = "flake-utils";
//   };
//
//   outputs = {
//     self,
//     nixpkgs,
//     flake-utils,
//     rust-overlay,
//     crane,
//   }:
//   {};
//   }
//     "#
//     }
//     fn minimal_flake_inputs_attrs() -> &'static str {
//         r#"
//         {
//   description = "flk - a tui for your flakes.";
//
//   inputs = {
//   nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
//
//   rust-overlay = {
//     url = "github:oxalica/rust-overlay";
//     inputs.nixpkgs.follows = "nixpkgs";
//     inputs.flake-utils.follows = "flake-utils";
//   };
//
//   crane = {
//     url = "github:ipetkov/crane";
//     inputs.nixpkgs.follows = "nixpkgs";
//     inputs.rust-overlay.follows = "rust-overlay";
//     inputs.flake-utils.follows = "flake-utils";
//   };
//   };
//
//   outputs = {
//     self,
//     nixpkgs,
//     flake-utils,
//     rust-overlay,
//     crane,
//   }:
//   {};
//   }
//     "#
//     }
//     fn only_inputs_flake() -> &'static str {
//         r#"
//         {
//   inputs.nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
//
//   inputs.rust-overlay = {
//     url = "github:oxalica/rust-overlay";
//     inputs.nixpkgs.follows = "nixpkgs";
//     inputs.flake-utils.follows = "flake-utils";
//   };
//
//   inputs.crane = {
//     url = "github:ipetkov/crane";
//     inputs.nixpkgs.follows = "nixpkgs";
//     inputs.rust-overlay.follows = "rust-overlay";
//     inputs.flake-utils.follows = "flake-utils";
//   };
//
//   outputs = {}:
//   {};
//   }
//     "#
//     }
//     fn no_inputs_flake() -> &'static str {
//         r#"
//         {
//   description = "flk - a tui for your flakes.";
//
//   outputs = {
//     self,
//     nixpkgs,
//   }:
//   {};
//   }
//     "#
//     }
// }
