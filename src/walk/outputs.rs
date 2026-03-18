use rnix::{SyntaxKind, SyntaxNode};

use crate::edit::{OutputChange, Outputs};

use super::error::WalkerError;
use super::node::parse_node;

/// Unwrap parentheses around a node, returning the inner node.
/// If the node is not NODE_PAREN, returns a clone of the original.
fn unwrap_parens(node: &SyntaxNode) -> SyntaxNode {
    if node.kind() == SyntaxKind::NODE_PAREN
        && let Some(inner) = node.children().find(|c| {
            c.kind() != SyntaxKind::TOKEN_L_PAREN && c.kind() != SyntaxKind::TOKEN_R_PAREN
        })
    {
        return inner;
    }
    node.clone()
}

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

            if let Some(next_sibling) = outputs_node.next_sibling() {
                let outputs_lambda = unwrap_parens(&next_sibling);
                if outputs_lambda.kind() != SyntaxKind::NODE_LAMBDA {
                    continue;
                }
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
///
/// Builds modifications bottom-up (output -> lambda -> toplevel -> attr_set)
/// then uses `attr_set.replace_with()` to propagate to NODE_ROOT,
/// preserving any leading comments/trivia.
pub fn change_outputs(
    root: &SyntaxNode,
    change: OutputChange,
) -> Result<Option<SyntaxNode>, WalkerError> {
    if root.kind() != SyntaxKind::NODE_ROOT {
        return Err(WalkerError::NotARoot(root.kind()));
    }

    let attr_set = root.first_child().unwrap();

    for toplevel in attr_set.children() {
        if toplevel.kind() == SyntaxKind::NODE_ATTRPATH_VALUE
            && let Some(outputs_node) = toplevel
                .children()
                .find(|child| child.to_string() == "outputs")
        {
            assert!(outputs_node.kind() == SyntaxKind::NODE_ATTRPATH);

            if let Some(next_sibling) = outputs_node.next_sibling() {
                let outputs_lambda = unwrap_parens(&next_sibling);
                if outputs_lambda.kind() != SyntaxKind::NODE_LAMBDA {
                    continue;
                }
                for output in outputs_lambda.children() {
                    if SyntaxKind::NODE_PATTERN == output.kind() {
                        if let OutputChange::Add(ref add) = change {
                            // Find the closing brace to insert before,
                            // accounting for any @-binding after the brace.
                            let r_brace_index = output
                                .children_with_tokens()
                                .position(|c| c.kind() == SyntaxKind::TOKEN_R_BRACE)
                                .expect("pattern must have closing brace");
                            // Insert before `}`, but if the token
                            // immediately before `}` is an entry or
                            // ellipsis (no whitespace gap), insert at
                            // `r_brace_index` so we go after it.
                            let before_brace = output
                                .children_with_tokens()
                                .nth(r_brace_index - 1)
                                .map(|c| c.kind());
                            let mut last_node = if matches!(
                                before_brace,
                                Some(
                                    SyntaxKind::NODE_PAT_ENTRY
                                        | SyntaxKind::TOKEN_ELLIPSIS
                                        | SyntaxKind::TOKEN_COMMA
                                )
                            ) {
                                r_brace_index
                            } else {
                                r_brace_index - 1
                            };

                            // Adjust the addition for trailing commas.
                            // Use the last NODE_PAT_ENTRY specifically, not
                            // the last child (which may be NODE_PAT_BIND for
                            // `@inputs` patterns).
                            let last_pat_entry = output
                                .children()
                                .filter(|c| c.kind() == SyntaxKind::NODE_PAT_ENTRY)
                                .last();
                            let has_trailing_comma = matches!(
                                last_pat_entry
                                    .as_ref()
                                    .and_then(|last| last.next_sibling_or_token())
                                    .map(|last_token| last_token.kind()),
                                Some(SyntaxKind::TOKEN_COMMA)
                            );

                            // Detect leading-comma style: commas appear before
                            // entries, preceded by whitespace containing a newline.
                            let leading_comma_ws = {
                                let tokens: Vec<_> = output.children_with_tokens().collect();
                                let mut result = None;
                                for i in 0..tokens.len() {
                                    if tokens[i].kind() == SyntaxKind::TOKEN_COMMA
                                        && i > 0
                                        && tokens[i - 1].kind() == SyntaxKind::TOKEN_WHITESPACE
                                        && tokens[i - 1].as_token().unwrap().text().contains('\n')
                                    {
                                        result = Some(
                                            tokens[i - 1].as_token().unwrap().text().to_string(),
                                        );
                                    }
                                }
                                result
                            };

                            // For leading-comma style, insert after the last
                            // entry rather than before `}`. This avoids double
                            // commas when there is a standalone trailing comma
                            // (e.g. `, flake-utils\n    ,\n    }`).
                            if leading_comma_ws.is_some() {
                                let mut last_entry_pos = None;
                                for (i, c) in output.children_with_tokens().enumerate() {
                                    if c.kind() == SyntaxKind::NODE_PAT_ENTRY {
                                        last_entry_pos = Some(i);
                                    }
                                }
                                if let Some(pos) = last_entry_pos {
                                    last_node = pos + 1;
                                }
                            }

                            // Detect multi-line trailing-comma style without
                            // trailing comma on the last entry: commas followed
                            // by whitespace containing newlines.
                            let multiline_trailing_ws = if !has_trailing_comma
                                && leading_comma_ws.is_none()
                            {
                                let tokens: Vec<_> = output.children_with_tokens().collect();
                                let mut result = None;
                                for i in 0..tokens.len() {
                                    if tokens[i].kind() == SyntaxKind::TOKEN_COMMA
                                        && i + 1 < tokens.len()
                                        && tokens[i + 1].kind() == SyntaxKind::TOKEN_WHITESPACE
                                        && tokens[i + 1].as_token().unwrap().text().contains('\n')
                                    {
                                        result = Some(
                                            tokens[i + 1].as_token().unwrap().text().to_string(),
                                        );
                                    }
                                }
                                result
                            } else {
                                None
                            };

                            let addition = if has_trailing_comma {
                                parse_node(&format!("{add},"))
                            } else if let Some(ref ws) = leading_comma_ws {
                                parse_node(&format!("{ws}, {add}"))
                            } else if let Some(ref ws) = multiline_trailing_ws {
                                parse_node(&format!(",{ws}{add}"))
                            } else {
                                parse_node(&format!(", {add}"))
                            };

                            let mut green = output
                                .green()
                                .insert_child(last_node, addition.green().into());
                            // Only insert whitespace before the addition when there's
                            // a trailing comma - the non-trailing-comma format already
                            // includes `, ` so extra whitespace would produce `x , y`.
                            if has_trailing_comma
                                && let Some(prev) =
                                    last_pat_entry.as_ref().unwrap().prev_sibling_or_token()
                                && let SyntaxKind::TOKEN_WHITESPACE = prev.kind()
                            {
                                let whitespace =
                                    parse_node(prev.as_token().unwrap().green().text());
                                green = green.insert_child(last_node, whitespace.green().into());
                            }
                            let changed_outputs_lambda = outputs_lambda
                                .green()
                                .replace_child(output.index(), green.into());
                            let changed_toplevel = if next_sibling.kind() == SyntaxKind::NODE_PAREN
                            {
                                let changed_paren = next_sibling.green().replace_child(
                                    outputs_lambda.index(),
                                    changed_outputs_lambda.into(),
                                );
                                toplevel
                                    .green()
                                    .replace_child(next_sibling.index(), changed_paren.into())
                            } else {
                                toplevel.green().replace_child(
                                    outputs_lambda.index(),
                                    changed_outputs_lambda.into(),
                                )
                            };
                            let changed_attr_set = attr_set
                                .green()
                                .replace_child(toplevel.index(), changed_toplevel.into());
                            let result = attr_set.replace_with(changed_attr_set);
                            return Ok(Some(parse_node(&result.to_string())));
                        }

                        for child in output.children() {
                            if child.kind() == SyntaxKind::NODE_PAT_ENTRY
                                && let OutputChange::Remove(ref id) = change
                                && child.to_string() == *id
                            {
                                let mut green = output.green().remove_child(child.index());
                                if let Some(prev) = child.prev_sibling_or_token()
                                    && let SyntaxKind::TOKEN_WHITESPACE = prev.kind()
                                {
                                    green = green.remove_child(prev.index());
                                    // Only remove the element before the whitespace
                                    // if it's a comma (non-first entry). When the
                                    // entry is first, the element before is `{` -
                                    // remove the trailing comma instead.
                                    if let Some(before_ws) = prev.prev_sibling_or_token()
                                        && before_ws.kind() == SyntaxKind::TOKEN_COMMA
                                    {
                                        green = green.remove_child(prev.index() - 1);
                                        // Leading-comma style: also remove the
                                        // newline+indent whitespace before the comma.
                                        if let Some(before_comma) =
                                            before_ws.prev_sibling_or_token()
                                            && before_comma.kind() == SyntaxKind::TOKEN_WHITESPACE
                                            && before_comma
                                                .as_token()
                                                .unwrap()
                                                .text()
                                                .contains('\n')
                                        {
                                            green = green.remove_child(prev.index() - 2);
                                        }
                                    } else {
                                        // First entry in leading-comma style:
                                        // remove the comma that belongs to
                                        // the next entry, along with the
                                        // whitespace between `{` and that
                                        // next entry.
                                        // After the two removals above,
                                        // prev.index() points to what was
                                        // right after the entry. Walk forward
                                        // from there and remove whitespace +
                                        // comma tokens until we hit the next
                                        // entry.
                                        let idx = prev.index();
                                        let children: Vec<_> = green.children().collect();
                                        let is_leading_comma = idx < children.len()
                                            && children[idx].kind().0
                                                == SyntaxKind::TOKEN_WHITESPACE as u16;
                                        drop(children);
                                        if is_leading_comma {
                                            // Leading-comma style first entry:
                                            // remove whitespace, comma, and
                                            // whitespace that belong to the
                                            // next entry, then re-insert a
                                            // space after `{`.
                                            loop {
                                                let children: Vec<_> = green.children().collect();
                                                if idx >= children.len() {
                                                    break;
                                                }
                                                let raw_kind = children[idx].kind().0;
                                                if raw_kind == SyntaxKind::TOKEN_WHITESPACE as u16
                                                    || raw_kind == SyntaxKind::TOKEN_COMMA as u16
                                                {
                                                    green = green.remove_child(idx);
                                                } else {
                                                    break;
                                                }
                                            }
                                            let ws = parse_node(" ");
                                            green = green.insert_child(idx, ws.green().into());
                                        } else {
                                            // Trailing-comma style first entry:
                                            // just remove the trailing comma.
                                            green = green.remove_child(idx);
                                        }
                                    }
                                } else {
                                    // No whitespace before the entry (prev is
                                    // `{` or absent). Remove trailing comma
                                    // and whitespace after the entry.
                                    let idx = child.index();
                                    loop {
                                        let children: Vec<_> = green.children().collect();
                                        if idx >= children.len() {
                                            break;
                                        }
                                        let raw_kind = children[idx].kind().0;
                                        if raw_kind == SyntaxKind::TOKEN_COMMA as u16
                                            || raw_kind == SyntaxKind::TOKEN_WHITESPACE as u16
                                        {
                                            green = green.remove_child(idx);
                                        } else {
                                            break;
                                        }
                                    }
                                }
                                let changed_outputs_lambda = outputs_lambda
                                    .green()
                                    .replace_child(output.index(), green.into());
                                let changed_toplevel = if next_sibling.kind()
                                    == SyntaxKind::NODE_PAREN
                                {
                                    let changed_paren = next_sibling.green().replace_child(
                                        outputs_lambda.index(),
                                        changed_outputs_lambda.into(),
                                    );
                                    toplevel
                                        .green()
                                        .replace_child(next_sibling.index(), changed_paren.into())
                                } else {
                                    toplevel.green().replace_child(
                                        outputs_lambda.index(),
                                        changed_outputs_lambda.into(),
                                    )
                                };
                                let changed_attr_set = attr_set
                                    .green()
                                    .replace_child(toplevel.index(), changed_toplevel.into());
                                let result = attr_set.replace_with(changed_attr_set);
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
