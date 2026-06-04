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
pub(crate) fn list_outputs(root: &SyntaxNode) -> Result<Outputs, WalkerError> {
    let mut outputs: Vec<String> = vec![];
    let mut any = false;

    if root.kind() != SyntaxKind::NODE_ROOT {
        return Err(WalkerError::NotARoot);
    }

    let Some(attr_set) = super::flake_attr_set(root) else {
        return Ok(Outputs::None);
    };

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

/// Whether a lambda parameter pattern ends its last `NODE_PAT_ENTRY` with
/// a comma. Looks past any `NODE_PAT_BIND` so `{ a, b }@inputs` is judged
/// by `b`, not by the binding.
fn has_trailing_comma(pattern: &SyntaxNode) -> bool {
    let last_pat_entry = pattern
        .children()
        .filter(|c| c.kind() == SyntaxKind::NODE_PAT_ENTRY)
        .last();
    matches!(
        last_pat_entry
            .as_ref()
            .and_then(|last| last.next_sibling_or_token())
            .map(|last_token| last_token.kind()),
        Some(SyntaxKind::TOKEN_COMMA)
    )
}

/// Detect leading-comma style: a comma preceded by whitespace that
/// contains a newline. Returns the whitespace text of the last such
/// match, so callers can mirror the indent recipe.
fn leading_comma_ws(pattern: &SyntaxNode) -> Option<String> {
    let tokens: Vec<_> = pattern.children_with_tokens().collect();
    let mut result = None;
    for i in 0..tokens.len() {
        if tokens[i].kind() == SyntaxKind::TOKEN_COMMA
            && i > 0
            && tokens[i - 1].kind() == SyntaxKind::TOKEN_WHITESPACE
            && tokens[i - 1].as_token().unwrap().text().contains('\n')
        {
            result = Some(tokens[i - 1].as_token().unwrap().text().to_string());
        }
    }
    result
}

/// Detect multi-line trailing-comma style: a comma followed by whitespace
/// that contains a newline. Returns the whitespace text of the last such
/// match. Callers gate this on `!has_trailing_comma && leading_comma_ws
/// .is_none()` to disambiguate from plain trailing-comma style.
fn multiline_trailing_ws(pattern: &SyntaxNode) -> Option<String> {
    let tokens: Vec<_> = pattern.children_with_tokens().collect();
    let mut result = None;
    for i in 0..tokens.len() {
        if tokens[i].kind() == SyntaxKind::TOKEN_COMMA
            && i + 1 < tokens.len()
            && tokens[i + 1].kind() == SyntaxKind::TOKEN_WHITESPACE
            && tokens[i + 1].as_token().unwrap().text().contains('\n')
        {
            result = Some(tokens[i + 1].as_token().unwrap().text().to_string());
        }
    }
    result
}

/// Bundled output of the three lambda-pattern style detectors, computed
/// once per pattern. `multiline_trailing_ws` is gated on
/// `!has_trailing_comma && leading_comma_ws.is_none()` to disambiguate
/// from plain trailing-comma style.
struct PatternStyle {
    has_trailing_comma: bool,
    leading_comma_ws: Option<String>,
    multiline_trailing_ws: Option<String>,
}

impl PatternStyle {
    fn detect(pattern: &SyntaxNode) -> Self {
        let trailing = has_trailing_comma(pattern);
        let leading = leading_comma_ws(pattern);
        let multi = if !trailing && leading.is_none() {
            multiline_trailing_ws(pattern)
        } else {
            None
        };
        Self {
            has_trailing_comma: trailing,
            leading_comma_ws: leading,
            multiline_trailing_ws: multi,
        }
    }
}

/// Insert `name` as a new `NODE_PAT_ENTRY` into `pattern`, mirroring the
/// pattern's existing comma/whitespace recipe.
fn add_output_arg(pattern: &SyntaxNode, name: &str, style: &PatternStyle) -> SyntaxNode {
    // Find the closing brace to insert before, accounting for any
    // @-binding after the brace.
    let r_brace_index = pattern
        .children_with_tokens()
        .position(|c| c.kind() == SyntaxKind::TOKEN_R_BRACE)
        .expect("pattern must have closing brace");
    // Insert before `}`, but if the token immediately before `}` is an
    // entry or ellipsis (no whitespace gap), insert at `r_brace_index`
    // so we go after it.
    let before_brace = pattern
        .children_with_tokens()
        .nth(r_brace_index - 1)
        .map(|c| c.kind());
    let mut last_node = if matches!(
        before_brace,
        Some(SyntaxKind::NODE_PAT_ENTRY | SyntaxKind::TOKEN_ELLIPSIS | SyntaxKind::TOKEN_COMMA)
    ) {
        r_brace_index
    } else {
        r_brace_index - 1
    };

    let last_pat_entry = pattern
        .children()
        .filter(|c| c.kind() == SyntaxKind::NODE_PAT_ENTRY)
        .last();

    // For leading-comma style, insert after the last entry rather than
    // before `}`. This avoids double commas when there is a standalone
    // trailing comma (e.g. `, flake-utils\n    ,\n    }`).
    if style.leading_comma_ws.is_some() {
        let mut last_entry_pos = None;
        for (i, c) in pattern.children_with_tokens().enumerate() {
            if c.kind() == SyntaxKind::NODE_PAT_ENTRY {
                last_entry_pos = Some(i);
            }
        }
        if let Some(pos) = last_entry_pos {
            last_node = pos + 1;
        }
    }

    let addition = if let Some(ref ws) = style.leading_comma_ws {
        parse_node(&format!("{ws}, {name}"))
    } else if style.has_trailing_comma {
        parse_node(&format!("{name},"))
    } else if let Some(ref ws) = style.multiline_trailing_ws {
        parse_node(&format!(",{ws}{name}"))
    } else {
        parse_node(&format!(", {name}"))
    };

    let mut green = pattern
        .green()
        .insert_child(last_node, addition.green().into());
    // Insert whitespace here only for trailing-comma style without
    // leading commas. Other formats already include spacing, and adding
    // more would produce `x , y`.
    if style.has_trailing_comma
        && style.leading_comma_ws.is_none()
        && let Some(prev) = last_pat_entry.as_ref().unwrap().prev_sibling_or_token()
        && let SyntaxKind::TOKEN_WHITESPACE = prev.kind()
    {
        let whitespace = parse_node(prev.as_token().unwrap().green().text());
        green = green.insert_child(last_node, whitespace.green().into());
    }
    SyntaxNode::new_root(green)
}

/// Locate the `NODE_PAT_ENTRY` child of `pattern` whose surface text equals
/// `name`. Returns `None` if no entry matches.
fn find_pat_entry_by_name(pattern: &SyntaxNode, name: &str) -> Option<SyntaxNode> {
    pattern
        .children()
        .find(|c| c.kind() == SyntaxKind::NODE_PAT_ENTRY && c.to_string() == name)
}

/// Remove the `NODE_PAT_ENTRY` whose text equals `name` from `pattern`,
/// stripping the surrounding comma and whitespace so the result stays
/// syntactically clean. Returns `None` if no matching entry exists.
fn remove_output_arg(
    pattern: &SyntaxNode,
    name: &str,
    _style: &PatternStyle,
) -> Option<SyntaxNode> {
    let child = find_pat_entry_by_name(pattern, name)?;
    let mut green = pattern.green().remove_child(child.index());

    let prev = child.prev_sibling_or_token();
    let prev_is_ws = prev
        .as_ref()
        .map(|p| p.kind() == SyntaxKind::TOKEN_WHITESPACE)
        .unwrap_or(false);
    if !prev_is_ws {
        // First entry, tight pattern `{self, ...}`: with no whitespace
        // before us, the comma and whitespace that separated us from
        // the next entry are still ahead and have to be cleared.
        let idx = child.index();
        while let Some(at_idx) = green.children().nth(idx) {
            let raw = at_idx.kind().0;
            if raw == SyntaxKind::TOKEN_COMMA as u16 || raw == SyntaxKind::TOKEN_WHITESPACE as u16 {
                green = green.remove_child(idx);
            } else {
                break;
            }
        }
        return Some(SyntaxNode::new_root(green));
    }
    let prev = prev.unwrap();
    green = green.remove_child(prev.index());

    if let Some(before_ws) = prev.prev_sibling_or_token()
        && before_ws.kind() == SyntaxKind::TOKEN_COMMA
    {
        // Non-first entry: drop the preceding comma. In leading-comma
        // style (`\n  , name`) the newline+indent ahead of the comma
        // belongs to this entry too; without it the result keeps a
        // stray blank line in place of the removed entry.
        green = green.remove_child(prev.index() - 1);
        if let Some(before_comma) = before_ws.prev_sibling_or_token()
            && before_comma.kind() == SyntaxKind::TOKEN_WHITESPACE
            && before_comma.as_token().unwrap().text().contains('\n')
        {
            green = green.remove_child(prev.index() - 2);
        }
        return Some(SyntaxNode::new_root(green));
    }

    // First entry, spaced pattern (`{ self, ...}` or `{ self\n, ...}`):
    // the comma sits with the *next* entry, not with us, so the cleanup
    // happens forward of where the entry used to be.
    let idx = prev.index();
    let next_is_ws = green
        .children()
        .nth(idx)
        .map(|c| c.kind().0 == SyntaxKind::TOKEN_WHITESPACE as u16)
        .unwrap_or(false);
    if next_is_ws {
        // Leading-comma style: the next entry's separator is multi-char
        // (`\n  ,`). Strip the run and put a single space back, otherwise
        // the result starts with `{\n  , next` which is broken syntax.
        while let Some(at_idx) = green.children().nth(idx) {
            let raw = at_idx.kind().0;
            if raw == SyntaxKind::TOKEN_WHITESPACE as u16 || raw == SyntaxKind::TOKEN_COMMA as u16 {
                green = green.remove_child(idx);
            } else {
                break;
            }
        }
        let ws = parse_node(" ");
        green = green.insert_child(idx, ws.green().into());
    } else {
        green = green.remove_child(idx);
    }
    Some(SyntaxNode::new_root(green))
}

/// Change the outputs attribute in a flake.nix root node.
///
/// Locates the `outputs = <lambda>` attribute, detects the lambda
/// pattern's style once, dispatches to [`add_output_arg`] or
/// [`remove_output_arg`], then rebuilds bottom-up
/// (pattern -> lambda -> toplevel -> attr_set) and uses
/// `attr_set.replace_with()` to propagate to NODE_ROOT, preserving any
/// leading comments/trivia.
pub(crate) fn change_outputs(
    root: &SyntaxNode,
    change: OutputChange,
) -> Result<Option<SyntaxNode>, WalkerError> {
    if root.kind() != SyntaxKind::NODE_ROOT {
        return Err(WalkerError::NotARoot);
    }

    let Some(attr_set) = super::flake_attr_set(root) else {
        return Ok(None);
    };

    for toplevel in attr_set.children() {
        if toplevel.kind() != SyntaxKind::NODE_ATTRPATH_VALUE {
            continue;
        }
        let Some(outputs_node) = toplevel
            .children()
            .find(|child| child.to_string() == "outputs")
        else {
            continue;
        };
        assert!(outputs_node.kind() == SyntaxKind::NODE_ATTRPATH);

        let Some(next_sibling) = outputs_node.next_sibling() else {
            continue;
        };
        let outputs_lambda = unwrap_parens(&next_sibling);
        if outputs_lambda.kind() != SyntaxKind::NODE_LAMBDA {
            continue;
        }
        let Some(pattern) = outputs_lambda
            .children()
            .find(|n| n.kind() == SyntaxKind::NODE_PATTERN)
        else {
            continue;
        };

        let style = PatternStyle::detect(&pattern);
        let new_pattern = match &change {
            OutputChange::Add(name) => Some(add_output_arg(&pattern, name, &style)),
            OutputChange::Remove(name) => remove_output_arg(&pattern, name, &style),
            OutputChange::None => None,
        };
        let Some(new_pattern) = new_pattern else {
            continue;
        };

        let changed_outputs_lambda = outputs_lambda
            .green()
            .replace_child(pattern.index(), new_pattern.green().into());
        let changed_toplevel = if next_sibling.kind() == SyntaxKind::NODE_PAREN {
            let changed_paren = next_sibling
                .green()
                .replace_child(outputs_lambda.index(), changed_outputs_lambda.into());
            toplevel
                .green()
                .replace_child(next_sibling.index(), changed_paren.into())
        } else {
            toplevel
                .green()
                .replace_child(outputs_lambda.index(), changed_outputs_lambda.into())
        };
        let changed_attr_set = attr_set
            .green()
            .replace_child(toplevel.index(), changed_toplevel.into());
        let result = attr_set.replace_with(changed_attr_set);
        return Ok(Some(parse_node(&result.to_string())));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pattern_from(src: &str) -> SyntaxNode {
        let root = rnix::Root::parse(src).syntax();
        root.descendants()
            .find(|n| n.kind() == SyntaxKind::NODE_PATTERN)
            .expect("input must contain a NODE_PATTERN")
    }

    #[test]
    fn trailing_comma_absent_in_single_line_pattern() {
        let p = pattern_from("{ self, nixpkgs }: {}");
        assert!(!has_trailing_comma(&p));
    }

    #[test]
    fn trailing_comma_present_in_single_line_pattern() {
        let p = pattern_from("{ self, nixpkgs, }: {}");
        assert!(has_trailing_comma(&p));
    }

    #[test]
    fn trailing_comma_present_in_multiline_pattern() {
        let p = pattern_from("{\n  self,\n  nixpkgs,\n}: {}");
        assert!(has_trailing_comma(&p));
    }

    #[test]
    fn trailing_comma_absent_in_multiline_pattern_without_one() {
        let p = pattern_from("{\n  self,\n  nixpkgs\n}: {}");
        assert!(!has_trailing_comma(&p));
    }

    #[test]
    fn trailing_comma_absent_in_leading_comma_style() {
        let p = pattern_from("{ self\n, nixpkgs\n}: {}");
        assert!(!has_trailing_comma(&p));
    }

    #[test]
    fn trailing_comma_judges_last_pat_entry_not_at_binding() {
        // `@inputs` adds a NODE_PAT_BIND child after `}`. The detector must
        // walk the last NODE_PAT_ENTRY (`nixpkgs`) and ignore the binding.
        let p = pattern_from("{ self, nixpkgs }@inputs: {}");
        assert!(!has_trailing_comma(&p));
    }

    #[test]
    fn leading_comma_ws_none_for_single_line_pattern() {
        let p = pattern_from("{ self, nixpkgs }: {}");
        assert_eq!(leading_comma_ws(&p), None);
    }

    #[test]
    fn leading_comma_ws_none_for_multiline_trailing_comma_style() {
        let p = pattern_from("{\n  self,\n  nixpkgs,\n}: {}");
        assert_eq!(leading_comma_ws(&p), None);
    }

    #[test]
    fn leading_comma_ws_some_for_leading_comma_style() {
        let p = pattern_from("{ self\n, nixpkgs\n}: {}");
        assert_eq!(leading_comma_ws(&p), Some("\n".to_string()));
    }

    #[test]
    fn leading_comma_ws_returns_last_match_ws_recipe() {
        let p = pattern_from("{ self\n  , nixpkgs\n  , flake-utils\n  }: {}");
        assert_eq!(leading_comma_ws(&p), Some("\n  ".to_string()));
    }

    #[test]
    fn multiline_trailing_ws_none_for_single_line_pattern() {
        let p = pattern_from("{ self, nixpkgs }: {}");
        assert_eq!(multiline_trailing_ws(&p), None);
    }

    #[test]
    fn multiline_trailing_ws_some_for_multiline_no_trailing_comma() {
        let p = pattern_from("{\n  self,\n  nixpkgs\n}: {}");
        assert_eq!(multiline_trailing_ws(&p), Some("\n  ".to_string()));
    }

    #[test]
    fn multiline_trailing_ws_returns_last_match_ws_recipe() {
        // The detector overwrites with each subsequent match, so a multi-line
        // trailing-comma pattern reports the whitespace after the LAST comma,
        // even though callers gate this case out.
        let p = pattern_from("{\n  self,\n  nixpkgs,\n}: {}");
        assert_eq!(multiline_trailing_ws(&p), Some("\n".to_string()));
    }

    #[test]
    fn multiline_trailing_ws_none_for_leading_comma_style() {
        let p = pattern_from("{ self\n, nixpkgs\n}: {}");
        assert_eq!(multiline_trailing_ws(&p), None);
    }

    #[test]
    fn detectors_agree_on_empty_pattern() {
        let p = pattern_from("{ }: {}");
        assert!(!has_trailing_comma(&p));
        assert_eq!(leading_comma_ws(&p), None);
        assert_eq!(multiline_trailing_ws(&p), None);
    }

    #[test]
    fn detectors_agree_on_single_entry_pattern() {
        let p = pattern_from("{ self }: {}");
        assert!(!has_trailing_comma(&p));
        assert_eq!(leading_comma_ws(&p), None);
        assert_eq!(multiline_trailing_ws(&p), None);
    }

    #[test]
    fn add_output_arg_appends_to_single_line_pattern() {
        let p = pattern_from("{ self, nixpkgs }: {}");
        let style = PatternStyle::detect(&p);
        let new_p = add_output_arg(&p, "flake-utils", &style);
        assert_eq!(new_p.to_string(), "{ self, nixpkgs, flake-utils }");
    }

    #[test]
    fn add_output_arg_preserves_multiline_trailing_comma_style() {
        let p = pattern_from("{\n  self,\n  nixpkgs,\n}: {}");
        let style = PatternStyle::detect(&p);
        let new_p = add_output_arg(&p, "flake-utils", &style);
        assert_eq!(
            new_p.to_string(),
            "{\n  self,\n  nixpkgs,\n  flake-utils,\n}"
        );
    }

    #[test]
    fn add_output_arg_preserves_multiline_no_trailing_comma_style() {
        let p = pattern_from("{\n  self,\n  nixpkgs\n}: {}");
        let style = PatternStyle::detect(&p);
        let new_p = add_output_arg(&p, "flake-utils", &style);
        assert_eq!(
            new_p.to_string(),
            "{\n  self,\n  nixpkgs,\n  flake-utils\n}"
        );
    }

    #[test]
    fn add_output_arg_preserves_leading_comma_style() {
        let p = pattern_from("{ self\n, nixpkgs\n}: {}");
        let style = PatternStyle::detect(&p);
        let new_p = add_output_arg(&p, "flake-utils", &style);
        assert_eq!(new_p.to_string(), "{ self\n, nixpkgs\n, flake-utils\n}");
    }

    #[test]
    fn find_pat_entry_by_name_returns_match() {
        let p = pattern_from("{ self, nixpkgs, flake-utils }: {}");
        let entry = find_pat_entry_by_name(&p, "nixpkgs").expect("entry must be found");
        assert_eq!(entry.kind(), SyntaxKind::NODE_PAT_ENTRY);
        assert_eq!(entry.to_string(), "nixpkgs");
    }

    #[test]
    fn find_pat_entry_by_name_returns_none_for_missing() {
        let p = pattern_from("{ self, nixpkgs }: {}");
        assert!(find_pat_entry_by_name(&p, "flake-utils").is_none());
    }

    #[test]
    fn find_pat_entry_by_name_does_not_match_binding() {
        // The `@inputs` binding is a NODE_PAT_BIND, not a NODE_PAT_ENTRY:
        // the helper must not return it even if the name matches.
        let p = pattern_from("{ self, nixpkgs }@inputs: {}");
        assert!(find_pat_entry_by_name(&p, "inputs").is_none());
    }

    #[test]
    fn remove_output_arg_removes_from_single_line_pattern() {
        let p = pattern_from("{ self, nixpkgs, flake-utils }: {}");
        let style = PatternStyle::detect(&p);
        let new_p = remove_output_arg(&p, "flake-utils", &style).expect("entry must be found");
        assert_eq!(new_p.to_string(), "{ self, nixpkgs }");
    }

    #[test]
    fn remove_output_arg_returns_none_for_missing_entry() {
        let p = pattern_from("{ self, nixpkgs }: {}");
        let style = PatternStyle::detect(&p);
        assert!(remove_output_arg(&p, "flake-utils", &style).is_none());
    }

    #[test]
    fn remove_output_arg_removes_from_multiline_trailing_comma_style() {
        let p = pattern_from("{\n  self,\n  nixpkgs,\n  flake-utils,\n}: {}");
        let style = PatternStyle::detect(&p);
        let new_p = remove_output_arg(&p, "flake-utils", &style).expect("entry must be found");
        assert_eq!(new_p.to_string(), "{\n  self,\n  nixpkgs,\n}");
    }

    #[test]
    fn remove_output_arg_removes_from_multiline_no_trailing_comma_style() {
        let p = pattern_from("{\n  self,\n  nixpkgs,\n  flake-utils\n}: {}");
        let style = PatternStyle::detect(&p);
        let new_p = remove_output_arg(&p, "flake-utils", &style).expect("entry must be found");
        assert_eq!(new_p.to_string(), "{\n  self,\n  nixpkgs\n}");
    }

    #[test]
    fn remove_output_arg_removes_from_leading_comma_style() {
        let p = pattern_from("{ self\n, nixpkgs\n, flake-utils\n}: {}");
        let style = PatternStyle::detect(&p);
        let new_p = remove_output_arg(&p, "flake-utils", &style).expect("entry must be found");
        assert_eq!(new_p.to_string(), "{ self\n, nixpkgs\n}");
    }

    #[test]
    fn remove_output_arg_removes_first_entry_from_tight_pattern() {
        // Tight, no whitespace between `{` and the first entry: covers
        // the `!prev_is_ws` branch where the comma+whitespace sit
        // forward of the removed entry.
        let p = pattern_from("{self, nixpkgs}: {}");
        let style = PatternStyle::detect(&p);
        let new_p = remove_output_arg(&p, "self", &style).expect("entry must be found");
        assert_eq!(new_p.to_string(), "{nixpkgs}");
    }

    #[test]
    fn remove_output_arg_removes_first_entry_from_spaced_pattern() {
        // Spaced first entry in trailing-comma style: covers the
        // first-entry branch where `next_is_ws` is false and only the
        // lone trailing comma needs to go.
        let p = pattern_from("{ self, nixpkgs }: {}");
        let style = PatternStyle::detect(&p);
        let new_p = remove_output_arg(&p, "self", &style).expect("entry must be found");
        assert_eq!(new_p.to_string(), "{ nixpkgs }");
    }

    #[test]
    fn remove_output_arg_removes_first_entry_from_leading_comma_style() {
        // First entry in leading-comma style: covers the branch that
        // strips the next entry's `\n  ,` separator and re-inserts a
        // single space so the result still parses.
        let p = pattern_from("{ self\n, nixpkgs\n, flake-utils\n}: {}");
        let style = PatternStyle::detect(&p);
        let new_p = remove_output_arg(&p, "self", &style).expect("entry must be found");
        assert_eq!(new_p.to_string(), "{ nixpkgs\n, flake-utils\n}");
    }
}
