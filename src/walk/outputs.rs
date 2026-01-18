use rnix::{SyntaxKind, SyntaxNode};

use crate::edit::{OutputChange, Outputs};

use super::error::WalkerError;
use super::node::parse_node;

/// List the outputs from a flake.nix root node.
pub fn list_outputs(root: &SyntaxNode) -> Result<Outputs, WalkerError> {
    let mut outputs: Vec<String> = vec![];
    let mut any = false;

    if root.kind() != SyntaxKind::NODE_ROOT {
        return Err(WalkerError::NotARoot(root.kind()));
    }

    for toplevel in root.first_child().unwrap().children() {
        if toplevel.kind() == SyntaxKind::NODE_ATTRPATH_VALUE
            && let Some(outputs_node) = toplevel
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

    if outputs.is_empty() {
        Ok(Outputs::None)
    } else if any {
        Ok(Outputs::Any(outputs))
    } else {
        Ok(Outputs::Multiple(outputs))
    }
}

/// Change the outputs attribute in a flake.nix root node.
pub fn change_outputs(
    root: &SyntaxNode,
    change: OutputChange,
) -> Result<Option<SyntaxNode>, WalkerError> {
    if root.kind() != SyntaxKind::NODE_ROOT {
        return Err(WalkerError::NotARoot(root.kind()));
    }

    for toplevel in root.first_child().unwrap().children() {
        if toplevel.kind() == SyntaxKind::NODE_ATTRPATH_VALUE
            && let Some(outputs_node) = toplevel
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

                            // Adjust the addition for trailing slashes
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
                                green = green.insert_child(last_node, whitespace.green().into());
                            }
                            let changed_outputs_lambda = outputs_lambda
                                .green()
                                .replace_child(output.index(), green.into());
                            let changed_toplevel = toplevel.green().replace_child(
                                outputs_lambda.index(),
                                changed_outputs_lambda.into(),
                            );
                            let result = root
                                .first_child()
                                .unwrap()
                                .green()
                                .replace_child(toplevel.index(), changed_toplevel.into());
                            return Ok(Some(parse_node(&result.to_string())));
                        }

                        for child in output.children() {
                            if child.kind() == SyntaxKind::NODE_PAT_ENTRY
                                && let OutputChange::Remove(ref id) = change
                                && child.to_string() == *id
                            {
                                let mut green = output.green().remove_child(child.index());
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
                                let result = root
                                    .first_child()
                                    .unwrap()
                                    .green()
                                    .replace_child(toplevel.index(), changed_toplevel.into());
                                return Ok(Some(parse_node(&result.to_string())));
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(None)
}
