//! CST walking and mutation for `flake.nix` files.

mod context;
mod error;
mod inputs;
mod node;
mod outputs;
mod record;
pub(crate) mod toggle;

use std::collections::HashMap;

use rnix::{Root, SyntaxKind, SyntaxNode};

use crate::change::Change;
use crate::edit::{OutputChange, Outputs};
use crate::follows::path::follows_idents_prefixed;
use crate::follows::{AttrPath, Segment, strip_outer_quotes};
use crate::input::Input;

pub(crate) use context::Context;
pub use error::WalkerError;

use inputs::{redeclare_removed_owner, retarget_existing_follows, walk_inputs};
use node::{
    FollowsKind, adjacent_whitespace_index, insert_binding_after, last_line_with_newline,
    make_toplevel_flake_false_attr, make_toplevel_url_attr, parse_node, substitute_child,
};

/// The flake's top-level attribute set.
///
/// A flake's root expression is normally a bare attribute set, but it may be
/// wrapped in `let <bindings> in { ... }`. In that case the inputs and outputs
/// live in the `in` body, not in the let bindings, so we descend through the
/// `NODE_LET_IN` to reach the body attrset and walk it exactly as a bare one.
///
/// Returns `None` when the root has no expression, or when a `let ... in` body
/// is not an attribute set (for example a function): such a flake exposes no
/// inputs or outputs to walk.
pub(crate) fn flake_attr_set(root: &SyntaxNode) -> Option<SyntaxNode> {
    let first = root.first_child()?;
    if first.kind() != SyntaxKind::NODE_LET_IN {
        return Some(first);
    }
    // `let <bindings> in <body>`: the body expression is the last child node,
    // after the bindings. Only an attrset body carries flake attributes.
    let body = first.last_child()?;
    (body.kind() == SyntaxKind::NODE_ATTR_SET).then_some(body)
}

fn is_flat_inputs_attr_for(idents: &[String], parent_id: &str) -> bool {
    idents.len() >= 2 && idents[0] == "inputs" && strip_outer_quotes(&idents[1]) == parent_id
}

fn block_parent_attrset(
    toplevel: &SyntaxNode,
    idents: &[String],
    parent_id: &str,
) -> Option<SyntaxNode> {
    if idents.len() != 2 {
        return None;
    }
    if !is_flat_inputs_attr_for(idents, parent_id) {
        return None;
    }
    toplevel
        .children()
        .find(|c| c.kind() == SyntaxKind::NODE_ATTR_SET)
}

/// Mirrors `ref_child`'s leading newline + indent so the inserted line
/// reads at the same column as its neighbour.
fn insert_flat_follows_after(
    attr_set: &SyntaxNode,
    ref_child: &SyntaxNode,
    path: &AttrPath,
    target: &str,
) -> SyntaxNode {
    let follows_node = FollowsKind::TopLevelNested { path, target }.emit();
    let new_attr_set = insert_binding_after(attr_set, ref_child, &follows_node);
    SyntaxNode::new_root(attr_set.replace_with(new_attr_set.green().to_owned()))
}

#[derive(Debug, Clone)]
pub struct Walker {
    pub(crate) root: SyntaxNode,
    pub(crate) inputs: HashMap<String, Input>,
    pub(crate) add_toplevel: bool,
}

impl<'a> Walker {
    pub fn new(stream: &'a str) -> Self {
        let root = Root::parse(stream).syntax();
        Self::from_root(root)
    }

    /// Build a walker around an already-parsed root, skipping the rnix parse.
    /// Lets callers that ran a parse for validation share the result.
    pub fn from_root(root: SyntaxNode) -> Self {
        Self {
            root,
            inputs: HashMap::new(),
            add_toplevel: false,
        }
    }

    /// Record the input model of the parsed `flake.nix` into `inputs`, then
    /// apply `change`, returning the rebuilt root if the tree was modified.
    /// `Change::None` only records.
    ///
    /// Expects the parsed root to be an attrset with `description`, `inputs`, and
    /// `outputs` keys.
    ///
    /// A `Change::Remove` acts on its first id only. Use
    /// [`crate::edit::FlakeEdit::apply_change`] to remove several.
    pub fn walk(&mut self, change: &Change) -> Result<Option<SyntaxNode>, WalkerError> {
        let cst = self.root.clone();
        if cst.kind() != SyntaxKind::NODE_ROOT {
            return Err(WalkerError::NotARoot);
        }
        let Some(attr_set) = flake_attr_set(&cst) else {
            self.inputs.clear();
            return Ok(None);
        };
        if let Some(stray) = attr_set
            .children()
            .find(|c| c.kind() != SyntaxKind::NODE_ATTRPATH_VALUE)
        {
            return Err(WalkerError::unexpected_top_level(
                &stray.to_string(),
                stray.text_range().start().into(),
            ));
        }

        self.inputs = record::record_inputs(&attr_set);
        if matches!(change, Change::None) {
            return Ok(None);
        }
        self.walk_toplevel(&attr_set, None, change)
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
        &self,
        attr_set: &SyntaxNode,
        ctx: Option<Context>,
        change: &Change,
    ) -> Result<Option<SyntaxNode>, WalkerError> {
        for toplevel in attr_set.children() {
            // Dispatch on the NODE_ATTRPATH child alone, not on the value.
            // For `inputs = { ... }` the value is the whole inputs attrset;
            // stringifying it dominates this walk on large flakes.
            let Some(attrpath) = toplevel
                .children()
                .find(|c| c.kind() == SyntaxKind::NODE_ATTRPATH)
            else {
                continue;
            };
            let mut path_idents = attrpath.children();
            let Some(first_ident) = path_idents.next() else {
                continue;
            };
            let has_more_idents = path_idents.next().is_some();
            let first_text = first_ident.to_string();
            let first_unquoted = strip_outer_quotes(&first_text);

            if !has_more_idents && first_unquoted == "description" {
                continue;
            }

            if first_unquoted == "inputs" {
                if has_more_idents {
                    if let Some(result) =
                        self.handle_inputs_flat(attr_set, &toplevel, &attrpath, &ctx, change)
                    {
                        return Ok(Some(result));
                    }
                } else if let Some(result) =
                    self.handle_inputs_attr(&toplevel, &attrpath, &ctx, change)
                {
                    return Ok(Some(result));
                }
                continue;
            }

            if !has_more_idents
                && first_unquoted == "outputs"
                && let Some(result) = self.handle_add_at_outputs(attr_set, &toplevel, change)
            {
                return Ok(Some(result));
            }
        }

        // Follows on toplevel flat-style inputs (`inputs.X.url = "..."`).
        if let Change::Follows { input, target } = change {
            let path = input.path();
            if path.len() >= 2 {
                let parent_id = input.input();
                if self.inputs.contains_key(parent_id.as_str()) {
                    let target_str = target.to_flake_follows_string();
                    return self.handle_follows_flat_toplevel(attr_set, path, &target_str);
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
        let expected_flat = follows_idents_prefixed(path.segments());

        if let Some(found) = retarget_existing_follows(attr_set, &expected_flat, target) {
            return Ok(match found {
                None => attr_set.ancestors().last(),
                Some(new_attr_set) => Some(SyntaxNode::new_root(
                    attr_set.replace_with(new_attr_set.green().to_owned()),
                )),
            });
        }

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

            if let Some(block_attr_set) =
                block_parent_attrset(&toplevel, &idents, parent_id.as_str())
            {
                block_parent = Some((toplevel.clone(), block_attr_set));
            }

            if is_flat_inputs_attr_for(&idents, parent_id.as_str()) {
                last_parent_attr = Some(toplevel.clone());
            }
        }

        if let Some((toplevel, block_attr_set)) = block_parent {
            let rest: Vec<Segment> = path.segments()[1..].to_vec();
            return Ok(Self::insert_follows_into_block_toplevel(
                attr_set,
                &toplevel,
                &block_attr_set,
                &rest,
                target,
            ));
        }

        if let Some(ref_child) = last_parent_attr {
            return Ok(Some(insert_flat_follows_after(
                attr_set, &ref_child, path, target,
            )));
        }

        Ok(None)
    }

    fn insert_follows_into_block_toplevel(
        attr_set: &SyntaxNode,
        toplevel: &SyntaxNode,
        block_attr_set: &SyntaxNode,
        rest: &[Segment],
        target: &str,
    ) -> Option<SyntaxNode> {
        let follows_node = FollowsKind::BlockNested { rest, target }.emit();
        if let Some(last_child) = block_attr_set.children().last() {
            let new_block = insert_binding_after(block_attr_set, &last_child, &follows_node);
            let new_toplevel = substitute_child(toplevel, block_attr_set.index(), &new_block);
            let green = attr_set
                .green()
                .replace_child(toplevel.index(), new_toplevel.green().to_owned().into());
            return Some(SyntaxNode::new_root(attr_set.replace_with(green)));
        }

        None
    }

    /// Apply `change` to the `inputs = { ... }` attribute.
    ///
    /// `toplevel.replace_with()` propagates through `NODE_ATTR_SET` up to `NODE_ROOT`,
    /// preserving leading comments and trivia.
    fn handle_inputs_attr(
        &self,
        toplevel: &SyntaxNode,
        child: &SyntaxNode,
        ctx: &Option<Context>,
        change: &Change,
    ) -> Option<SyntaxNode> {
        let sibling = child.next_sibling()?;
        let replacement = walk_inputs(sibling.clone(), ctx, change)?;

        let green = toplevel
            .green()
            .replace_child(sibling.index(), replacement.green().to_owned().into());
        let green = toplevel.replace_with(green);
        Some(SyntaxNode::new_root(green))
    }

    /// Apply `change` to flat-style `inputs.foo.url = "..."` attributes.
    ///
    /// Removals rebuild the parent attrset green and `replace_with()` propagates to
    /// `NODE_ROOT`. Replacements rely on `toplevel.replace_with()` to propagate.
    fn handle_inputs_flat(
        &self,
        attr_set: &SyntaxNode,
        toplevel: &SyntaxNode,
        child: &SyntaxNode,
        ctx: &Option<Context>,
        change: &Change,
    ) -> Option<SyntaxNode> {
        let replacement = walk_inputs(child.clone(), ctx, change)?;

        if replacement.to_string().is_empty()
            && let Some(redeclared) = redeclare_removed_owner(toplevel, change)
        {
            let green = attr_set
                .green()
                .replace_child(toplevel.index(), redeclared.green().to_owned().into());
            return Some(SyntaxNode::new_root(attr_set.replace_with(green)));
        }

        // Empty replacement means we remove the entire toplevel node and
        // propagate through attr_set to NODE_ROOT.
        if replacement.to_string().is_empty() {
            let element: rnix::SyntaxElement = toplevel.clone().into();
            let mut green = attr_set.green().remove_child(toplevel.index());
            if let Some(ws_index) = adjacent_whitespace_index(&element) {
                green = green.remove_child(ws_index);
            }
            return Some(SyntaxNode::new_root(attr_set.replace_with(green)));
        }

        let sibling = child.next_sibling()?;
        let green = toplevel
            .green()
            .replace_child(sibling.index(), replacement.green().to_owned().into());
        let green = toplevel.replace_with(green);
        Some(SyntaxNode::new_root(green))
    }

    /// Add a new input just before `outputs` when no `inputs` block exists yet.
    ///
    /// Rebuilds the parent attrset green. `replace_with()` propagates to `NODE_ROOT`
    /// while preserving leading comments.
    fn handle_add_at_outputs(
        &self,
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
        let id = id.input();

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
                    ws = Some(parse_node(last_line_with_newline(&ws_str)));
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
            .insert_child(insert_pos, addition.green().to_owned().into());

        if let Some(ref ws) = ws_node {
            green = green.insert_child(insert_pos, ws.green().to_owned().into());
        }

        // Append `inputs.<id>.flake = false;` when the new input opts out of flake mode.
        if !flake {
            let no_flake = make_toplevel_flake_false_attr(id);
            green = green.insert_child(toplevel.index() + 1, no_flake.green().to_owned().into());

            if let Some(ref ws) = ws_node {
                green = green.insert_child(toplevel.index() + 1, ws.green().to_owned().into());
            }
        }

        Some(SyntaxNode::new_root(attr_set.replace_with(green)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::change::{Change, ChangeId};
    use crate::follows::AttrPath;

    fn apply(flake_text: &str, change: &Change) -> String {
        let mut walker = Walker::new(flake_text);
        walker
            .walk(change)
            .expect("walker error")
            .expect("walker did not rewrite the tree")
            .to_string()
    }

    fn follows_change(input: &str, target: &str) -> Change {
        Change::Follows {
            input: ChangeId::parse(input).unwrap(),
            target: AttrPath::parse(target).unwrap(),
        }
    }

    #[test]
    fn handle_follows_flat_toplevel_inserts_follows_after_last_parent_attr() {
        let flake = "{
  inputs.flake-edit.url = \"github:a-kenji/flake-edit\";
  inputs.nixpkgs.url = \"github:NixOS/nixpkgs\";

  outputs = { self, ... }: { };
}
";
        let result = apply(flake, &follows_change("flake-edit.nixpkgs", "nixpkgs"));
        assert_eq!(
            result,
            "{
  inputs.flake-edit.url = \"github:a-kenji/flake-edit\";
  inputs.flake-edit.inputs.nixpkgs.follows = \"nixpkgs\";
  inputs.nixpkgs.url = \"github:NixOS/nixpkgs\";

  outputs = { self, ... }: { };
}
"
        );
    }

    #[test]
    fn handle_follows_flat_toplevel_retargets_existing_follows() {
        let flake = "{
  inputs.flake-edit.url = \"github:a-kenji/flake-edit\";
  inputs.flake-edit.inputs.nixpkgs.follows = \"old-pkgs\";
  inputs.nixpkgs.url = \"github:NixOS/nixpkgs\";

  outputs = { self, ... }: { };
}
";
        let result = apply(flake, &follows_change("flake-edit.nixpkgs", "nixpkgs"));
        assert_eq!(
            result,
            "{
  inputs.flake-edit.url = \"github:a-kenji/flake-edit\";
  inputs.flake-edit.inputs.nixpkgs.follows = \"nixpkgs\";
  inputs.nixpkgs.url = \"github:NixOS/nixpkgs\";

  outputs = { self, ... }: { };
}
"
        );
    }

    #[test]
    fn handle_follows_flat_toplevel_is_noop_when_target_already_matches() {
        let flake = "{
  inputs.flake-edit.url = \"github:a-kenji/flake-edit\";
  inputs.flake-edit.inputs.nixpkgs.follows = \"nixpkgs\";
  inputs.nixpkgs.url = \"github:NixOS/nixpkgs\";

  outputs = { self, ... }: { };
}
";
        let result = apply(flake, &follows_change("flake-edit.nixpkgs", "nixpkgs"));
        assert_eq!(result, flake);
    }

    #[test]
    fn handle_follows_flat_toplevel_delegates_to_block_parent_when_present() {
        let flake = "{
  inputs.nixpkgs.url = \"github:NixOS/nixpkgs\";
  inputs.flake-edit = {
    url = \"github:a-kenji/flake-edit\";
  };

  outputs = { self, ... }: { };
}
";
        let result = apply(flake, &follows_change("flake-edit.nixpkgs", "nixpkgs"));
        assert_eq!(
            result,
            "{
  inputs.nixpkgs.url = \"github:NixOS/nixpkgs\";
  inputs.flake-edit = {
    url = \"github:a-kenji/flake-edit\";
    inputs.nixpkgs.follows = \"nixpkgs\";
  };

  outputs = { self, ... }: { };
}
"
        );
    }

    #[test]
    fn is_flat_inputs_attr_for_only_matches_matching_parent_id() {
        let yes = [
            "inputs".to_string(),
            "flake-edit".to_string(),
            "url".to_string(),
        ];
        let no = [
            "inputs".to_string(),
            "nixpkgs".to_string(),
            "url".to_string(),
        ];
        assert!(is_flat_inputs_attr_for(&yes, "flake-edit"));
        assert!(!is_flat_inputs_attr_for(&no, "flake-edit"));
        // The CST keeps surrounding `"..."` on quoted idents; the comparison
        // must unquote them, otherwise `"flake-edit" != flake-edit`.
        let quoted = [
            "inputs".to_string(),
            "\"flake-edit\"".to_string(),
            "url".to_string(),
        ];
        assert!(is_flat_inputs_attr_for(&quoted, "flake-edit"));
    }
}
