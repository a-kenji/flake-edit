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
    adjacent_whitespace_index, get_sibling_whitespace, make_toplevel_flake_false_attr,
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

        let addition = make_toplevel_url_attr(id, uri);
        let insert_pos = toplevel.index() - 1;

        let mut green = attr_set
            .green()
            .insert_child(insert_pos, addition.green().into());

        // Add whitespace before the new input
        if let Some(prev_child) = attr_set
            .children()
            .find(|c| c.index() == toplevel.index() - 2)
            && let Some(whitespace) = get_sibling_whitespace(&prev_child)
        {
            green = green.insert_child(insert_pos, whitespace.green().into());
        }

        // Add flake=false if needed
        if !flake {
            let no_flake = make_toplevel_flake_false_attr(id);
            green = green.insert_child(toplevel.index() + 1, no_flake.green().into());

            if let Some(prev_child) = attr_set
                .children()
                .find(|c| c.index() == toplevel.index() - 2)
                && let Some(whitespace) = get_sibling_whitespace(&prev_child)
            {
                green = green.insert_child(toplevel.index() + 1, whitespace.green().into());
            }
        }

        Some(parse_node(&attr_set.replace_with(green).to_string()))
    }
}
