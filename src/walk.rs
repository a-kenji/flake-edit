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

use inputs::{remove_child_with_whitespace, walk_inputs};
use node::{
    get_sibling_whitespace, make_toplevel_flake_false_attr, make_toplevel_follows_attr,
    make_toplevel_url_attr, parse_node,
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
        let cst = &self.root;
        if cst.kind() != SyntaxKind::NODE_ROOT {
            return Err(WalkerError::NotARoot(cst.kind()));
        }
        self.walk_toplevel(cst.clone(), None, change)
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
        let Some(root) = node.first_child() else {
            return Ok(None);
        };

        for toplevel in root.children() {
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
                    if let Some(result) =
                        self.handle_inputs_attr(&root, &toplevel, &child, &ctx, change)
                    {
                        return Ok(Some(result));
                    }
                    continue;
                }

                if child_str.starts_with("inputs") {
                    if let Some(result) =
                        self.handle_inputs_flat(&root, &toplevel, &child, &ctx, change)
                    {
                        return Ok(Some(result));
                    }
                    continue;
                }

                if child_str == "outputs"
                    && let Some(result) =
                        self.handle_add_at_outputs(&root, &toplevel, &child, change)
                {
                    return Ok(Some(result));
                }
            }
        }
        Ok(None)
    }

    /// Handle `inputs = { ... }` attribute
    fn handle_inputs_attr(
        &mut self,
        _root: &SyntaxNode,
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

    /// Handle flat-style `inputs.foo.url = "..."` attributes
    fn handle_inputs_flat(
        &mut self,
        root: &SyntaxNode,
        toplevel: &SyntaxNode,
        child: &SyntaxNode,
        ctx: &Option<Context>,
        change: &Change,
    ) -> Option<SyntaxNode> {
        let replacement = walk_inputs(&mut self.inputs, child.clone(), ctx, change)?;

        // If replacement is empty, remove the entire toplevel node
        if replacement.to_string().is_empty() {
            return Some(remove_child_with_whitespace(
                root,
                toplevel,
                toplevel.index(),
            ));
        }

        let sibling = child.next_sibling()?;
        let green = toplevel
            .green()
            .replace_child(sibling.index(), replacement.green().into());
        let green = toplevel.replace_with(green);
        Some(parse_node(&green.to_string()))
    }

    /// Handle adding inputs when we've reached `outputs` but have no inputs yet
    fn handle_add_at_outputs(
        &mut self,
        root: &SyntaxNode,
        toplevel: &SyntaxNode,
        child: &SyntaxNode,
        change: &Change,
    ) -> Option<SyntaxNode> {
        if !self.add_toplevel {
            return None;
        }

        let Change::Add {
            id: Some(id),
            uri: Some(uri),
            flake,
            follows,
        } = change
        else {
            return None;
        };

        if toplevel.index() == 0 {
            return None;
        }

        let addition = make_toplevel_url_attr(id, uri);
        let insert_pos = toplevel.index() - 1;

        let mut green = root
            .green()
            .insert_child(insert_pos, addition.green().into());

        // Add whitespace before the new input
        if let Some(prev_child) = root.children().find(|c| c.index() == toplevel.index() - 2)
            && let Some(whitespace) = get_sibling_whitespace(&prev_child)
        {
            green = green.insert_child(insert_pos, whitespace.green().into());
        }

        // Position for additional attributes (follows, flake=false)
        // Using toplevel.index() + 1 as the base position for items after the url,
        // since inserting at the same position pushes previous items forward
        let after_url_pos = toplevel.index() + 1;

        // Add follows directives
        for follow_spec in follows {
            let follows_node = make_toplevel_follows_attr(id, &follow_spec.from, &follow_spec.to);
            green = green.insert_child(after_url_pos, follows_node.green().into());

            if let Some(prev_child) = root.children().find(|c| c.index() == toplevel.index() - 2)
                && let Some(whitespace) = get_sibling_whitespace(&prev_child)
            {
                green = green.insert_child(after_url_pos, whitespace.green().into());
            }
        }

        // Add flake=false if needed
        if !flake {
            let no_flake = make_toplevel_flake_false_attr(id);
            green = green.insert_child(after_url_pos, no_flake.green().into());

            if let Some(prev_child) = root.children().find(|c| c.index() == toplevel.index() - 2)
                && let Some(whitespace) = get_sibling_whitespace(&prev_child)
            {
                green = green.insert_child(after_url_pos, whitespace.green().into());
            }
        }

        // Preserve whitespace after outputs
        if let Some(next) = child.next_sibling_or_token()
            && next.kind() == SyntaxKind::TOKEN_WHITESPACE
        {
            let whitespace = parse_node(next.as_token().unwrap().green().text());
            green = green.insert_child(child.index() + 1, whitespace.green().into());
        }

        Some(parse_node(&green.to_string()))
    }
}
