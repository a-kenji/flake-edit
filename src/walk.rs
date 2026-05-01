//! CST walking and mutation for `flake.nix` files.

mod context;
mod error;
mod inputs;
mod node;
mod outputs;

use std::collections::HashMap;

use rnix::{Root, SyntaxKind, SyntaxNode};

use crate::change::Change;
use crate::edit::{OutputChange, Outputs};
use crate::follows::{AttrPath, Segment, strip_outer_quotes};
use crate::input::Input;

pub use context::Context;
pub use error::WalkerError;

use inputs::walk_inputs;
use node::{
    FollowsKind, adjacent_whitespace_index, get_sibling_whitespace, insertion_index_after,
    make_quoted_string, make_toplevel_flake_false_attr, make_toplevel_url_attr, parse_node,
    substitute_child,
};

/// Expected idents for a top-level flat follows attrpath of any depth:
/// `inputs.<S0>.inputs.<S1>...inputs.<SN>.follows`.
fn expected_toplevel_flat_idents(path: &AttrPath) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::with_capacity(path.len() * 2 + 2);
    for seg in path.segments() {
        out.push("inputs");
        out.push(seg.as_str());
    }
    out.push("follows");
    out
}

/// Expected idents for a block-style follows attrpath inside a parent input's
/// `{ ... }` block: `inputs.<R0>.inputs.<R1>...inputs.<RN>.follows`.
fn expected_block_follows_idents(rest: &[Segment]) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::with_capacity(rest.len() * 2 + 1);
    for seg in rest.iter() {
        out.push("inputs");
        out.push(seg.as_str());
    }
    out.push("follows");
    out
}

/// Whether a CST attrpath (idents may carry surrounding `"..."`) matches `expected`
/// pairwise after unquoting.
fn idents_match(have: &[String], expected: &[&str]) -> bool {
    if have.len() != expected.len() {
        return false;
    }
    have.iter()
        .zip(expected.iter())
        .all(|(h, e)| strip_outer_quotes(h) == *e)
}

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

    /// Apply `change` to the parsed `flake.nix`, returning the rebuilt root if
    /// the tree was modified.
    ///
    /// Expects the parsed root to be an attrset with `description`, `inputs`, and
    /// `outputs` keys.
    pub fn walk(&mut self, change: &Change) -> Result<Option<SyntaxNode>, WalkerError> {
        let cst = self.root.clone();
        if cst.kind() != SyntaxKind::NODE_ROOT {
            return Err(WalkerError::NotARoot(cst.kind()));
        }
        self.walk_toplevel(cst, None, change)
    }

    /// List the `outputs` arguments without touching `inputs`.
    pub(crate) fn list_outputs(&mut self) -> Result<Outputs, WalkerError> {
        outputs::list_outputs(&self.root)
    }

    /// Apply an [`OutputChange`] to the `outputs` attribute alone.
    pub(crate) fn change_outputs(
        &mut self,
        change: OutputChange,
    ) -> Result<Option<SyntaxNode>, WalkerError> {
        outputs::change_outputs(&self.root, change)
    }

    /// Walk the top-level attrset, dispatching on `description`/`inputs`/`outputs`.
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

        // Follows on toplevel flat-style inputs (`inputs.X.url = "..."`).
        if let Change::Follows { input, target } = change {
            let path = input.path();
            if path.len() >= 2 {
                let parent_id = input.input();
                if self.inputs.contains_key(parent_id.as_str()) {
                    let target_str = target.to_string();
                    return self.handle_follows_flat_toplevel(&attr_set, path, &target_str);
                }
            }
        }

        Ok(None)
    }

    /// Add a follows attribute next to a toplevel flat-style input.
    ///
    /// Converts `inputs.crane.url = "github:...";` into:
    /// ```nix
    /// inputs.crane.url = "github:...";
    /// inputs.crane.inputs.nixpkgs.follows = "nixpkgs";
    /// ```
    fn handle_follows_flat_toplevel(
        &self,
        attr_set: &SyntaxNode,
        path: &AttrPath,
        target: &str,
    ) -> Result<Option<SyntaxNode>, WalkerError> {
        let parent_id = path.first();
        // Toplevel-flat shape: `inputs.S0.inputs.S1...inputs.SN.follows`
        // (2 * len + 1 idents).
        let expected_flat = expected_toplevel_flat_idents(path);
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
                && strip_outer_quotes(&idents[1]) == parent_id.as_str()
                && let Some(block_attr_set) = toplevel
                    .children()
                    .find(|c| c.kind() == SyntaxKind::NODE_ATTR_SET)
            {
                block_parent = Some((toplevel.clone(), block_attr_set));
            }

            // Check for existing follows: inputs.S0.inputs.S1...inputs.SN.follows
            if idents_match(&idents, &expected_flat) {
                let value_node = attrpath.next_sibling();
                let current_target = value_node
                    .as_ref()
                    .map(|v| strip_outer_quotes(&v.to_string()).to_string())
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
            if idents.len() >= 2
                && idents[0] == "inputs"
                && strip_outer_quotes(&idents[1]) == parent_id.as_str()
            {
                last_parent_attr = Some(toplevel.clone());
            }
        }

        if let Some((toplevel, block_attr_set)) = block_parent {
            let rest: Vec<Segment> = path.segments()[1..].to_vec();
            return self.handle_follows_block_toplevel(
                attr_set,
                &toplevel,
                &block_attr_set,
                &rest,
                target,
            );
        }

        // No existing follows, insert after the last parent attribute
        if let Some(ref_child) = last_parent_attr {
            let follows_node = FollowsKind::TopLevelNested { path, target }.emit();
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
        rest: &[Segment],
        target: &str,
    ) -> Result<Option<SyntaxNode>, WalkerError> {
        let expected_block = expected_block_follows_idents(rest);
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

            if idents_match(&idents, &expected_block) {
                let value_node = attrpath.next_sibling();
                let current_target = value_node
                    .as_ref()
                    .map(|v| strip_outer_quotes(&v.to_string()).to_string())
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

        let follows_node = FollowsKind::BlockNested { rest, target }.emit();
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

    /// Apply `change` to the `inputs = { ... }` attribute.
    ///
    /// `toplevel.replace_with()` propagates through `NODE_ATTR_SET` up to `NODE_ROOT`,
    /// preserving leading comments and trivia.
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

    /// Apply `change` to flat-style `inputs.foo.url = "..."` attributes.
    ///
    /// Removals rebuild the parent attrset green and `replace_with()` propagates to
    /// `NODE_ROOT`. Replacements rely on `toplevel.replace_with()` to propagate.
    fn handle_inputs_flat(
        &mut self,
        attr_set: &SyntaxNode,
        toplevel: &SyntaxNode,
        child: &SyntaxNode,
        ctx: &Option<Context>,
        change: &Change,
    ) -> Option<SyntaxNode> {
        let replacement = walk_inputs(&mut self.inputs, child.clone(), ctx, change)?;

        // Empty replacement means we remove the entire toplevel node and
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

    /// Add a new input just before `outputs` when no `inputs` block exists yet.
    ///
    /// Rebuilds the parent attrset green. `replace_with()` propagates to `NODE_ROOT`
    /// while preserving leading comments.
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

        // Walk back from `outputs` through tokens to find a whitespace run, then
        // normalize it to a single newline + indent. Walking through tokens (not
        // siblings) lets us skip past comments between the last input and `outputs`.
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

        // Append `inputs.<id>.flake = false;` when the new input opts out of flake mode.
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
