use std::collections::HashMap;

use rnix::{Root, SyntaxKind, SyntaxNode};

use crate::{
    change::Change,
    edit::{OutputChange, Outputs},
    input::Input,
};

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
    fn new(level: Vec<String>) -> Self {
        Self { level }
    }

    pub fn level(&self) -> Vec<String> {
        self.level.clone()
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

    /// Only walk the outputs attribute
    pub(crate) fn list_outputs(&mut self) -> Outputs {
        let mut outputs: Vec<String> = vec![];
        let mut any = false;
        tracing::debug!("Walking outputs.");
        let cst = &self.root;
        if cst.kind() != SyntaxKind::NODE_ROOT {
            // TODO: handle this as an error
            panic!("Should be a topevel node.")
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
            Outputs::None
        } else if any {
            Outputs::Any(outputs)
        } else {
            Outputs::Multiple(outputs)
        }
    }
    /// Only change the outputs attribute
    pub(crate) fn change_outputs(&mut self, change: OutputChange) -> Option<SyntaxNode> {
        tracing::debug!("Changing outputs.");
        let cst = &self.root;
        if cst.kind() != SyntaxKind::NODE_ROOT {
            // TODO: handle this as an error
            panic!("Should be a toplevel node.")
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
                                            .and_then(|last_token| Some(last_token.kind()))
                                        {
                                            Root::parse(&format!("{add},")).syntax()
                                        } else {
                                            Root::parse(&format!(", {add}")).syntax()
                                        };

                                        let mut green = output
                                            .green()
                                            .insert_child(last_node, addition.green().into());
                                        if let Some(prev) = output
                                            .children()
                                            .nth(count - 1)
                                            .unwrap()
                                            .prev_sibling_or_token()
                                        {
                                            if let SyntaxKind::TOKEN_WHITESPACE = prev.kind() {
                                                let whitespace = Root::parse(
                                                    prev.as_token().unwrap().green().text(),
                                                )
                                                .syntax();
                                                green = green.insert_child(
                                                    last_node,
                                                    whitespace.green().into(),
                                                );
                                            }
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
                                        return Some(Root::parse(&result.to_string()).syntax());
                                    }

                                    for child in output.children() {
                                        if child.kind() == SyntaxKind::NODE_PAT_ENTRY {
                                            if let OutputChange::Remove(ref id) = change {
                                                if child.to_string() == *id {
                                                    let mut green =
                                                        output.green().remove_child(child.index());
                                                    if let Some(prev) =
                                                        child.prev_sibling_or_token()
                                                    {
                                                        if let SyntaxKind::TOKEN_WHITESPACE =
                                                            prev.kind()
                                                        {
                                                            green =
                                                                green.remove_child(prev.index());
                                                            green = green
                                                                .remove_child(prev.index() - 1);
                                                        }
                                                    } else if let Some(next) =
                                                        child.next_sibling_or_token()
                                                    {
                                                        if let SyntaxKind::TOKEN_WHITESPACE =
                                                            next.kind()
                                                        {
                                                            green =
                                                                green.remove_child(next.index());
                                                        }
                                                    }
                                                    let changed_outputs_lambda =
                                                        outputs_lambda.green().replace_child(
                                                            output.index(),
                                                            green.into(),
                                                        );
                                                    let changed_toplevel =
                                                        toplevel.green().replace_child(
                                                            outputs_lambda.index(),
                                                            changed_outputs_lambda.into(),
                                                        );
                                                    let result = cst
                                                        .first_child()
                                                        .unwrap()
                                                        .green()
                                                        .replace_child(
                                                            toplevel.index(),
                                                            changed_toplevel.into(),
                                                        );
                                                    return Some(
                                                        Root::parse(&result.to_string()).syntax(),
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
            }
        }
        None
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
                                    let node = Self::remove_child_with_whitespace(
                                        &root,
                                        &toplevel,
                                        toplevel.index(),
                                    );
                                    return Some(node);
                                } else {
                                    tracing::debug!("Replacement Node: {replacement}");
                                    let green = toplevel.green().replace_child(
                                        child.next_sibling().unwrap().index(),
                                        replacement.green().into(),
                                    );
                                    let green = toplevel.replace_with(green);
                                    let node = Root::parse(green.to_string().as_str()).syntax();
                                    return Some(node);
                                }
                            }
                        };
                        // If we already see outputs, but have no inputs
                        // we need to create a toplevel inputs attribute set.
                        if child.to_string() == "outputs" && self.add_toplevel {
                            if let Change::Add { id, uri, flake } = change {
                                let addition = Root::parse(&format!(
                                    "inputs.{}.url = \"{}\";",
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
                                                    let whitespace = Root::parse(
                                                        prev.as_token().unwrap().green().text(),
                                                    )
                                                    .syntax();
                                                    node = node.insert_child(
                                                        toplevel.index() - 1,
                                                        whitespace.green().into(),
                                                    );
                                                }
                                            } else if let Some(next) = c.next_sibling_or_token() {
                                                if next.kind() == SyntaxKind::TOKEN_WHITESPACE {
                                                    let whitespace = Root::parse(
                                                        next.as_token().unwrap().green().text(),
                                                    )
                                                    .syntax();
                                                    node = node.insert_child(
                                                        toplevel.index() - 1,
                                                        whitespace.green().into(),
                                                    );
                                                }
                                            }
                                        });
                                    if !flake {
                                        let no_flake = Root::parse(&format!(
                                            "inputs.{}.flake = false;",
                                            id.clone().unwrap(),
                                        ))
                                        .syntax();
                                        node = node.insert_child(
                                            toplevel.index() + 1,
                                            no_flake.green().into(),
                                        );
                                        root.children()
                                            .find(|c| c.index() == toplevel.index() - 2)
                                            .map(|c| {
                                                if let Some(prev) = c.prev_sibling_or_token() {
                                                    if prev.kind() == SyntaxKind::TOKEN_WHITESPACE {
                                                        let whitespace = Root::parse(
                                                            prev.as_token().unwrap().green().text(),
                                                        )
                                                        .syntax();
                                                        node = node.insert_child(
                                                            toplevel.index() + 1,
                                                            whitespace.green().into(),
                                                        );
                                                    }
                                                } else if let Some(next) = c.next_sibling_or_token()
                                                {
                                                    if next.kind() == SyntaxKind::TOKEN_WHITESPACE {
                                                        let whitespace = Root::parse(
                                                            next.as_token().unwrap().green().text(),
                                                        )
                                                        .syntax();
                                                        node = node.insert_child(
                                                            toplevel.index() + 1,
                                                            whitespace.green().into(),
                                                        );
                                                    }
                                                }
                                            });
                                    }
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
                    // TODO: proper error handling.
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
                if let Some(follows_id) = &maybe_follows_id {
                    let maybe_input_id = node
                        .children()
                        .find(|child| child.to_string() == "inputs")
                        .and_then(|input_child| input_child.next_sibling());
                    let ctx = maybe_input_id
                        .clone()
                        .map(|id| Context::new(vec![id.to_string()]));
                    let mut input = Input::new(follows_id.to_string());
                    input.url = node.next_sibling().unwrap().to_string();
                    let text_range = node.next_sibling().unwrap().text_range();
                    input.range = crate::input::Range::from_text_range(text_range);
                    self.insert_with_ctx(follows_id.to_string(), input, &ctx);

                    // Remove a toplevel follows node
                    if let Some(input_id) = maybe_input_id {
                        if change.is_remove() {
                            if let Some(id) = change.id() {
                                let maybe_follows = maybe_follows_id.map(|id| id.to_string());
                                if id.matches_with_follows(&input_id.to_string(), maybe_follows) {
                                    let replacement = Root::parse("").syntax();
                                    return Some(replacement);
                                }
                            }
                        }
                    }
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
                    } else if change.is_some() && change.id().is_some() && ctx.is_none() {
                        if let Change::Add { id, uri, flake } = change {
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
                            if !flake {
                                let no_flake = Root::parse(&format!(
                                    "{}.flake = false;",
                                    id.clone().unwrap(),
                                ))
                                .syntax();
                                green =
                                    green.insert_child(child.index() + 2, no_flake.green().into());
                                if prev.kind() == SyntaxKind::TOKEN_WHITESPACE {
                                    let whitespace =
                                        Root::parse(prev.as_token().unwrap().green().text())
                                            .syntax();
                                    green = green
                                        .insert_child(child.index() + 3, whitespace.green().into());
                                }
                            }
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
                                                        let text_range = url.text_range();
                                                        input.range =
                                                            crate::input::Range::from_text_range(
                                                                text_range,
                                                            );
                                                        self.insert_with_ctx(
                                                            next_sibling.to_string(),
                                                            input,
                                                            ctx,
                                                        );
                                                        if change.is_some() && change.is_remove() {
                                                            if let Some(id) = change.id() {
                                                                if id.to_string()
                                                                    == next_sibling.to_string()
                                                                {
                                                                    let replacement =
                                                                        Root::parse("").syntax();
                                                                    tracing::debug!("Node: {node}");
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
                                                } else if url_id.to_string() == "flake" {
                                                    if let Some(is_flake) = child
                                                        .as_node()
                                                        .unwrap()
                                                        .parent()
                                                        .unwrap()
                                                        .next_sibling()
                                                    {
                                                        tracing::debug!(
                                                            "This id {} is a flake: {}",
                                                            next_sibling,
                                                            is_flake
                                                        );
                                                        // let mut input =
                                                        //     Input::new(next_sibling.to_string());
                                                        // input.flake =
                                                        //     is_flake.to_string().parse().unwrap();
                                                        // self.insert_with_ctx(
                                                        //     next_sibling.to_string(),
                                                        //     input,
                                                        //     ctx,
                                                        // );
                                                        if change.is_some() && change.is_remove() {
                                                            if let Some(id) = change.id() {
                                                                if id.to_string()
                                                                    == next_sibling.to_string()
                                                                {
                                                                    let replacement =
                                                                        Root::parse("").syntax();
                                                                    tracing::debug!("Node: {node}");
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
                                                        let text_range = next_sibling.text_range();
                                                        input.range =
                                                            crate::input::Range::from_text_range(
                                                                text_range,
                                                            );
                                                        self.insert_with_ctx(
                                                            next_sibling.to_string(),
                                                            input,
                                                            ctx,
                                                        );
                                                    }
                                                    if change.is_remove() {
                                                        if let Some(id) = change.id() {
                                                            if id.to_string()
                                                                == next_sibling.to_string()
                                                            {
                                                                let replacement =
                                                                    Root::parse("").syntax();
                                                                tracing::debug!("Node: {node}");
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
                                                tracing::debug!(
                                                    "Walking inputs with: {attr}, context: {context:?}"
                                                );
                                                if let Some(change) =
                                                    self.walk_input(&attr, &Some(context), change)
                                                {
                                                    tracing::debug!("Adjusted change: {change}");
                                                    tracing::debug!(
                                                        "Adjusted change is_empty: {}",
                                                        change.to_string().is_empty()
                                                    );
                                                    tracing::debug!(
                                                        "Child index: {}",
                                                        child.index()
                                                    );
                                                    tracing::debug!(
                                                        "Child node: {}",
                                                        child.as_node().unwrap()
                                                    );
                                                    tracing::debug!("Nested Attr: {}", nested_attr);
                                                    tracing::debug!("Node: {}", node);
                                                    tracing::debug!("Attr: {}", attr);
                                                    // TODO: adjust node correctly if the change is
                                                    // not empty
                                                    let replacement =
                                                        Self::remove_child_with_whitespace(
                                                            &nested_attr,
                                                            &attr,
                                                            attr.index(),
                                                        );
                                                    tracing::debug!("Replacement: {}", replacement);
                                                    return Some(replacement);
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
                                if id.to_string() == prev_id.to_string() {
                                    tracing::debug!("Removing: {id}");
                                    let empty = Root::parse("").syntax();
                                    return Some(empty);
                                }
                            }
                            if let Some(sibling) = child.next_sibling() {
                                tracing::debug!("This is an url from {} - {}", prev_id, sibling);
                                let mut input = Input::new(prev_id.to_string());
                                input.url = sibling.to_string();
                                let text_range = sibling.text_range();
                                input.range = crate::input::Range::from_text_range(text_range);
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
                                                        tracing::debug!(
                                                            "The following attribute follows: {id}:{follows} is nested inside the attr: {ctx:?}"
                                                        );
                                                        let mut input = Input::new(id.to_string());
                                                        input.url = follows.to_string();
                                                        let text_range = follows.text_range();
                                                        input.range =
                                                            crate::input::Range::from_text_range(
                                                                text_range,
                                                            );
                                                        self.insert_with_ctx(
                                                            id.to_string(),
                                                            input,
                                                            ctx,
                                                        );
                                                        if change.is_remove() {
                                                            if let Some(id) = change.id() {
                                                                if id.matches_with_ctx(
                                                                    &follows.to_string(),
                                                                    ctx.clone(),
                                                                ) {
                                                                    let replacement =
                                                                        Root::parse("").syntax();
                                                                    return Some(replacement);
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if attr.to_string() == "flake" {
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
                                if change.is_remove() {
                                    if let Some(id) = change.id() {
                                        if id.matches_with_ctx(&input_id.to_string(), ctx.clone()) {
                                            let replacement = Root::parse("").syntax();
                                            return Some(replacement);
                                        }
                                    }
                                }
                            }
                        } else {
                            // TODO: handle this.
                            // This happens, when there is a nested node.
                            tracing::info!("Nested: This is not handled yet.");
                        }
                    }
                    if attr.to_string() == "follows" {
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
                        let mut input = Input::new(id.to_string());
                        input.url = follows.to_string();
                        let text_range = follows.text_range();
                        input.range = crate::input::Range::from_text_range(text_range);
                        self.insert_with_ctx(id.to_string(), input.clone(), ctx);
                        if let Some(id) = change.id() {
                            if let Some(ctx) = ctx {
                                if id.matches_with_ctx(input.id(), Some(ctx.clone()))
                                    && change.is_remove()
                                {
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
                            let text_range = uri.text_range();
                            input.range = crate::input::Range::from_text_range(text_range);
                            self.insert_with_ctx(id.to_string(), input, ctx);

                            // Remove matched node.
                            if let Change::Remove { id: candidate } = change {
                                if candidate.to_string() == id.to_string() {
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
                            {
                                // TODO: adjustment of whitespace, if node is empty
                                // TODO: if it leaves an empty attr, then remove whole?
                                let tree = node
                                    .green()
                                    .replace_child(child.index(), replacement.green().into());
                                let replacement = Root::parse(&tree.to_string()).syntax();
                                return Some(replacement);
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
