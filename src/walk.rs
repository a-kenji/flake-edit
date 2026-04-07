//! AST walking and manipulation for flake.nix files.

mod context;
mod error;
mod inputs;
mod node;
mod outputs;

use std::collections::HashMap;

use rnix::{Root, SyntaxKind, SyntaxNode};

use crate::change::Change;
use crate::edit::{OutputChange, Outputs};
use crate::input::Input;

pub use context::Context;
pub use error::WalkerError;

use inputs::walk_inputs;
use node::{
    adjacent_whitespace_index, get_sibling_whitespace, insertion_index_after,
    make_nested_follows_attr, make_quoted_string, make_toplevel_flake_false_attr,
    make_toplevel_nested_follows_attr, make_toplevel_url_attr, parse_node, substitute_child,
};

#[derive(Debug, Clone)]
pub struct Walker {
    pub root: SyntaxNode,
    pub inputs: HashMap<String, Input>,
    pub add_toplevel: bool,
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
        let cst = self.root.clone();
        if cst.kind() != SyntaxKind::NODE_ROOT {
            return Err(WalkerError::NotARoot(cst.kind()));
        }
        self.walk_toplevel(cst, None, change)
    }

    /// Only walk the outputs attribute
    pub(crate) fn list_outputs(&mut self) -> Result<Outputs, WalkerError> {
        outputs::list_outputs(&self.root)
    }

    /// Only change the outputs attribute
    pub(crate) fn change_outputs(
        &mut self,
        change: OutputChange,
    ) -> Result<Option<SyntaxNode>, WalkerError> {
        outputs::change_outputs(&self.root, change)
    }

    /// Traverse the toplevel `flake.nix` file.
    fn walk_toplevel(
        &mut self,
        node: SyntaxNode,
        ctx: Option<Context>,
        change: &Change,
    ) -> Result<Option<SyntaxNode>, WalkerError> {
        let Some(attr_set) = node.first_child() else {
            return Ok(None);
        };

        for toplevel in attr_set.children() {
            if toplevel.kind() != SyntaxKind::NODE_ATTRPATH_VALUE {
                return Err(WalkerError::UnexpectedNodeKind {
                    expected: SyntaxKind::NODE_ATTRPATH_VALUE,
                    found: toplevel.kind(),
                });
            }

            for child in toplevel.children() {
                let child_str = child.to_string();

                if child_str == "description" {
                    break;
                }

                if child_str == "inputs" {
                    if let Some(result) = self.handle_inputs_attr(&toplevel, &child, &ctx, change) {
                        return Ok(Some(result));
                    }
                    continue;
                }

                if child_str.starts_with("inputs") {
                    if let Some(result) =
                        self.handle_inputs_flat(&attr_set, &toplevel, &child, &ctx, change)
                    {
                        return Ok(Some(result));
                    }
                    continue;
                }

                if child_str == "outputs"
                    && let Some(result) = self.handle_add_at_outputs(&attr_set, &toplevel, change)
                {
                    return Ok(Some(result));
                }
            }
        }

        // Handle follows for toplevel flat-style inputs (inputs.X.url = "...")
        if let Change::Follows { input, target } = change
            && let Some(nested_id) = input.follows()
        {
            let parent_id = input.input();
            if self.inputs.contains_key(parent_id) {
                return self.handle_follows_flat_toplevel(&attr_set, parent_id, nested_id, target);
            }
        }

        Ok(None)
    }

    /// Handle adding follows to a toplevel flat-style input.
    ///
    /// Converts `inputs.crane.url = "github:...";` into:
    /// ```nix
    /// inputs.crane.url = "github:...";
    /// inputs.crane.inputs.nixpkgs.follows = "nixpkgs";
    /// ```
    fn handle_follows_flat_toplevel(
        &self,
        attr_set: &SyntaxNode,
        parent_id: &str,
        nested_id: &str,
        target: &str,
    ) -> Result<Option<SyntaxNode>, WalkerError> {
        let mut last_parent_attr: Option<SyntaxNode> = None;
        let mut block_parent: Option<(SyntaxNode, SyntaxNode)> = None;

        for toplevel in attr_set.children() {
            if toplevel.kind() != SyntaxKind::NODE_ATTRPATH_VALUE {
                continue;
            }
            let Some(attrpath) = toplevel
                .children()
                .find(|c| c.kind() == SyntaxKind::NODE_ATTRPATH)
            else {
                continue;
            };
            let idents: Vec<String> = attrpath.children().map(|c| c.to_string()).collect();

            // Detect `inputs.{parent_id} = { ... }` block style
            if idents.len() == 2
                && idents[0] == "inputs"
                && idents[1] == parent_id
                && let Some(block_attr_set) = toplevel
                    .children()
                    .find(|c| c.kind() == SyntaxKind::NODE_ATTR_SET)
            {
                block_parent = Some((toplevel.clone(), block_attr_set));
            }

            // Check for existing follows: inputs.{parent_id}.inputs.{nested_id}.follows
            if idents.len() == 5
                && idents[0] == "inputs"
                && idents[1] == parent_id
                && idents[2] == "inputs"
                && idents[3] == nested_id
                && idents[4] == "follows"
            {
                let value_node = attrpath.next_sibling();
                let current_target = value_node
                    .as_ref()
                    .map(|v| v.to_string().trim_matches('"').to_string())
                    .unwrap_or_default();

                if current_target == target {
                    // Same target, no-op
                    return Ok(Some(parse_node(&attr_set.parent().unwrap().to_string())));
                }
                // Different target, retarget
                if let Some(value) = value_node {
                    let new_value = make_quoted_string(target);
                    let new_toplevel = substitute_child(&toplevel, value.index(), &new_value);
                    let green = attr_set
                        .green()
                        .replace_child(toplevel.index(), new_toplevel.green().into());
                    return Ok(Some(parse_node(&attr_set.replace_with(green).to_string())));
                }
            }

            // Track last inputs.{parent_id}.* attribute
            if idents.len() >= 2 && idents[0] == "inputs" && idents[1] == parent_id {
                last_parent_attr = Some(toplevel.clone());
            }
        }

        if let Some((toplevel, block_attr_set)) = block_parent {
            return self.handle_follows_block_toplevel(
                attr_set,
                &toplevel,
                &block_attr_set,
                nested_id,
                target,
            );
        }

        // No existing follows, insert after the last parent attribute
        if let Some(ref_child) = last_parent_attr {
            let follows_node = make_toplevel_nested_follows_attr(parent_id, nested_id, target);
            let insert_index = insertion_index_after(&ref_child);

            let mut green = attr_set
                .green()
                .insert_child(insert_index, follows_node.green().into());

            if let Some(whitespace) = get_sibling_whitespace(&ref_child) {
                let ws_str = whitespace.to_string();
                let normalized = if let Some(last_nl) = ws_str.rfind('\n') {
                    &ws_str[last_nl..]
                } else {
                    &ws_str
                };
                let ws_node = parse_node(normalized);
                green = green.insert_child(insert_index, ws_node.green().into());
            }

            return Ok(Some(parse_node(&attr_set.replace_with(green).to_string())));
        }

        Ok(None)
    }

    fn handle_follows_block_toplevel(
        &self,
        attr_set: &SyntaxNode,
        toplevel: &SyntaxNode,
        block_attr_set: &SyntaxNode,
        nested_id: &str,
        target: &str,
    ) -> Result<Option<SyntaxNode>, WalkerError> {
        for attr in block_attr_set.children() {
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

            if idents.len() == 3
                && idents[0] == "inputs"
                && idents[1] == nested_id
                && idents[2] == "follows"
            {
                let value_node = attrpath.next_sibling();
                let current_target = value_node
                    .as_ref()
                    .map(|v| v.to_string().trim_matches('"').to_string())
                    .unwrap_or_default();

                if current_target == target {
                    return Ok(Some(parse_node(&attr_set.parent().unwrap().to_string())));
                }

                if let Some(value) = value_node {
                    let new_value = make_quoted_string(target);
                    let new_attr = substitute_child(&attr, value.index(), &new_value);
                    let new_block = substitute_child(block_attr_set, attr.index(), &new_attr);
                    let new_toplevel =
                        substitute_child(toplevel, block_attr_set.index(), &new_block);
                    let green = attr_set
                        .green()
                        .replace_child(toplevel.index(), new_toplevel.green().into());
                    return Ok(Some(parse_node(&attr_set.replace_with(green).to_string())));
                }
            }
        }

        let follows_node = make_nested_follows_attr(nested_id, target);
        let children: Vec<_> = block_attr_set.children().collect();
        if let Some(last_child) = children.last() {
            let insert_index = last_child.index() + 1;

            let mut green = block_attr_set
                .green()
                .insert_child(insert_index, follows_node.green().into());

            if let Some(whitespace) = get_sibling_whitespace(last_child) {
                green = green.insert_child(insert_index, whitespace.green().into());
            }

            let new_block = parse_node(&green.to_string());
            let new_toplevel = substitute_child(toplevel, block_attr_set.index(), &new_block);
            let green = attr_set
                .green()
                .replace_child(toplevel.index(), new_toplevel.green().into());
            return Ok(Some(parse_node(&attr_set.replace_with(green).to_string())));
        }

        Ok(None)
    }

    /// Handle `inputs = { ... }` attribute.
    ///
    /// `toplevel.replace_with()` propagates through NODE_ATTR_SET up to
    /// NODE_ROOT, preserving any leading comments/trivia.
    fn handle_inputs_attr(
        &mut self,
        toplevel: &SyntaxNode,
        child: &SyntaxNode,
        ctx: &Option<Context>,
        change: &Change,
    ) -> Option<SyntaxNode> {
        let sibling = child.next_sibling()?;
        let replacement = walk_inputs(&mut self.inputs, sibling.clone(), ctx, change)?;

        let green = toplevel
            .green()
            .replace_child(sibling.index(), replacement.green().into());
        let green = toplevel.replace_with(green);
        Some(parse_node(&green.to_string()))
    }

    /// Handle flat-style `inputs.foo.url = "..."` attributes.
    ///
    /// For removals, builds the modified attr_set green and uses
    /// `replace_with()` to propagate to NODE_ROOT.
    /// For replacements, `toplevel.replace_with()` propagates naturally.
    fn handle_inputs_flat(
        &mut self,
        attr_set: &SyntaxNode,
        toplevel: &SyntaxNode,
        child: &SyntaxNode,
        ctx: &Option<Context>,
        change: &Change,
    ) -> Option<SyntaxNode> {
        let replacement = walk_inputs(&mut self.inputs, child.clone(), ctx, change)?;

        // If replacement is empty, remove the entire toplevel node and
        // propagate through attr_set to NODE_ROOT.
        if replacement.to_string().is_empty() {
            let element: rnix::SyntaxElement = toplevel.clone().into();
            let mut green = attr_set.green().remove_child(toplevel.index());
            if let Some(ws_index) = adjacent_whitespace_index(&element) {
                green = green.remove_child(ws_index);
            }
            return Some(parse_node(&attr_set.replace_with(green).to_string()));
        }

        let sibling = child.next_sibling()?;
        let green = toplevel
            .green()
            .replace_child(sibling.index(), replacement.green().into());
        let green = toplevel.replace_with(green);
        Some(parse_node(&green.to_string()))
    }

    /// Handle adding inputs when we've reached `outputs` but have no inputs yet.
    ///
    /// Builds the modified attr_set green and uses `replace_with()` to
    /// propagate to NODE_ROOT, preserving leading comments.
    fn handle_add_at_outputs(
        &mut self,
        attr_set: &SyntaxNode,
        toplevel: &SyntaxNode,
        change: &Change,
    ) -> Option<SyntaxNode> {
        if !self.add_toplevel {
            return None;
        }

        let Change::Add {
            id: Some(id),
            uri: Some(uri),
            flake,
        } = change
        else {
            return None;
        };

        if toplevel.index() == 0 {
            return None;
        }

        // Find normalized whitespace (single newline + indent) by walking back
        // from `outputs` through tokens. This handles comments between the last
        // input and outputs correctly.
        let ws_node = {
            let mut ws: Option<SyntaxNode> = None;
            let mut cursor = toplevel.prev_sibling_or_token();
            while let Some(ref tok) = cursor {
                if tok.kind() == SyntaxKind::TOKEN_WHITESPACE {
                    let ws_str = tok.to_string();
                    let normalized = if let Some(last_nl) = ws_str.rfind('\n') {
                        &ws_str[last_nl..]
                    } else {
                        &ws_str
                    };
                    ws = Some(parse_node(normalized));
                    break;
                }
                cursor = tok.prev_sibling_or_token();
            }
            ws
        };

        let addition = make_toplevel_url_attr(id, uri);
        let insert_pos = toplevel.index() - 1;

        let mut green = attr_set
            .green()
            .insert_child(insert_pos, addition.green().into());

        if let Some(ref ws) = ws_node {
            green = green.insert_child(insert_pos, ws.green().into());
        }

        // Add flake=false if needed
        if !flake {
            let no_flake = make_toplevel_flake_false_attr(id);
            green = green.insert_child(toplevel.index() + 1, no_flake.green().into());

            if let Some(ref ws) = ws_node {
                green = green.insert_child(toplevel.index() + 1, ws.green().into());
            }
        }

        Some(parse_node(&attr_set.replace_with(green).to_string()))
    }
}
