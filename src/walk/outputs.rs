use rnix::{Root, SyntaxKind, SyntaxNode};
use rowan::{GreenNode, NodeOrToken};

use crate::edit::{OutputChange, Outputs};

use super::error::WalkerError;
use super::node::{comma_token, whitespace_token};

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
            assert_eq!(outputs_node.kind(), SyntaxKind::NODE_ATTRPATH);

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

    let entry = NodeOrToken::Node(parse_pat_entry(name));
    let comma = NodeOrToken::Token(comma_token());
    let ws = |text: &str| NodeOrToken::Token(whitespace_token(text));
    let addition = if let Some(ref lead) = style.leading_comma_ws {
        vec![ws(lead), comma, ws(" "), entry]
    } else if style.has_trailing_comma {
        vec![entry, comma]
    } else if let Some(ref trail) = style.multiline_trailing_ws {
        vec![comma, ws(trail), entry]
    } else {
        vec![comma, ws(" "), entry]
    };

    let mut green = pattern.green().to_owned();
    for (offset, element) in addition.into_iter().enumerate() {
        green = green.insert_child(last_node + offset, element);
    }
    // Insert whitespace here only for trailing-comma style without
    // leading commas. Other formats already include spacing, and adding
    // more would produce `x , y`.
    if style.has_trailing_comma
        && style.leading_comma_ws.is_none()
        && let Some(prev) = last_pat_entry.as_ref().unwrap().prev_sibling_or_token()
        && let SyntaxKind::TOKEN_WHITESPACE = prev.kind()
    {
        green = green.insert_child(last_node, whitespace_token(&prev.to_string()).into());
    }
    SyntaxNode::new_root(green)
}

fn parse_pat_entry(name: &str) -> GreenNode {
    Root::parse(&format!("{{ {name} }}: null"))
        .syntax()
        .descendants()
        .find(|n| n.kind() == SyntaxKind::NODE_PAT_ENTRY)
        .unwrap_or_else(|| panic!("not a pattern entry: {name:?}"))
        .green()
        .to_owned()
}

/// Locate the `NODE_PAT_ENTRY` child of `pattern` whose surface text equals
/// `name`. Returns `None` if no entry matches.
fn find_pat_entry_by_name(pattern: &SyntaxNode, name: &str) -> Option<SyntaxNode> {
    pattern
        .children()
        .find(|c| c.kind() == SyntaxKind::NODE_PAT_ENTRY && c.to_string() == name)
}

/// Remove the entry `name` and one adjacent comma from `pattern`, keeping every comment.
fn remove_output_arg(pattern: &SyntaxNode, name: &str) -> Option<SyntaxNode> {
    use SyntaxKind::{TOKEN_COMMA, TOKEN_COMMENT, TOKEN_WHITESPACE};

    let entry = find_pat_entry_by_name(pattern, name)?;
    let kids: Vec<rnix::SyntaxElement> = pattern.children_with_tokens().collect();
    let kind = |i: usize| kids[i].kind();
    let is_trivia = |i: usize| matches!(kind(i), TOKEN_WHITESPACE | TOKEN_COMMENT);
    let at = entry.index();

    let next_comma = (at + 1..kids.len())
        .find(|&i| !is_trivia(i))
        .filter(|&i| kind(i) == TOKEN_COMMA);
    let prev_comma = (0..at)
        .rev()
        .find(|&i| !is_trivia(i))
        .filter(|&i| kind(i) == TOKEN_COMMA);
    let comment_after_prev_comma =
        prev_comma.is_some_and(|p| (p + 1..at).any(|i| kind(i) == TOKEN_COMMENT));
    let drop_next_comma =
        next_comma.is_some() && (prev_comma.is_none() || comment_after_prev_comma);
    let removed = match (drop_next_comma, prev_comma, next_comma) {
        (true, _, Some(next)) => at..=next,
        (false, Some(prev), _) => prev..=at,
        _ => at..=at,
    };

    let mut keep: Vec<bool> = (0..kids.len())
        .map(|i| {
            !removed.contains(&i)
                || kind(i) == TOKEN_COMMENT
                || (kind(i) == TOKEN_WHITESPACE
                    && (kind(i - 1) == TOKEN_COMMENT
                        || kids.get(i + 1).is_some_and(|k| k.kind() == TOKEN_COMMENT)))
        })
        .collect();
    let end = *removed.end();
    let entry_is_tight = kind(at - 1) != TOKEN_WHITESPACE;
    if drop_next_comma
        && entry_is_tight
        && end + 1 < kids.len()
        && kind(end + 1) == TOKEN_WHITESPACE
    {
        keep[end + 1] = false;
    }

    let kept: Vec<usize> = (0..kids.len()).filter(|&i| keep[i]).collect();
    for (w, seam) in kept.windows(2).enumerate() {
        let (earlier, later) = (seam[0], seam[1]);
        if kind(earlier) != TOKEN_WHITESPACE || kind(later) != TOKEN_WHITESPACE {
            continue;
        }
        let later_breaks_line = kids[later].to_string().contains('\n');
        let later_is_entry_side = !drop_next_comma;
        let earlier_ends_comment = w > 0 && kind(kept[w - 1]) == TOKEN_COMMENT;
        let keep_later = later_breaks_line || (later_is_entry_side && !earlier_ends_comment);
        keep[if keep_later { earlier } else { later }] = false;
    }

    let mut green = pattern.green().to_owned();
    for i in (0..kids.len()).rev().filter(|&i| !keep[i]) {
        green = green.remove_child(i);
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
        assert_eq!(outputs_node.kind(), SyntaxKind::NODE_ATTRPATH);

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
            OutputChange::Remove(name) => remove_output_arg(&pattern, name),
            OutputChange::None => None,
        };
        let Some(new_pattern) = new_pattern else {
            continue;
        };

        let changed_outputs_lambda = outputs_lambda
            .green()
            .replace_child(pattern.index(), new_pattern.green().to_owned().into());
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
        return Ok(Some(SyntaxNode::new_root(result)));
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
        let new_p = remove_output_arg(&p, "flake-utils").expect("entry must be found");
        assert_eq!(new_p.to_string(), "{ self, nixpkgs }");
    }

    #[test]
    fn remove_output_arg_returns_none_for_missing_entry() {
        let p = pattern_from("{ self, nixpkgs }: {}");
        assert!(remove_output_arg(&p, "flake-utils").is_none());
    }

    #[test]
    fn remove_output_arg_removes_from_multiline_trailing_comma_style() {
        let p = pattern_from("{\n  self,\n  nixpkgs,\n  flake-utils,\n}: {}");
        let new_p = remove_output_arg(&p, "flake-utils").expect("entry must be found");
        assert_eq!(new_p.to_string(), "{\n  self,\n  nixpkgs,\n}");
    }

    #[test]
    fn remove_output_arg_removes_from_multiline_no_trailing_comma_style() {
        let p = pattern_from("{\n  self,\n  nixpkgs,\n  flake-utils\n}: {}");
        let new_p = remove_output_arg(&p, "flake-utils").expect("entry must be found");
        assert_eq!(new_p.to_string(), "{\n  self,\n  nixpkgs\n}");
    }

    #[test]
    fn remove_output_arg_removes_from_leading_comma_style() {
        let p = pattern_from("{ self\n, nixpkgs\n, flake-utils\n}: {}");
        let new_p = remove_output_arg(&p, "flake-utils").expect("entry must be found");
        assert_eq!(new_p.to_string(), "{ self\n, nixpkgs\n}");
    }

    #[test]
    fn remove_output_arg_removes_first_entry_from_tight_pattern() {
        // Tight, no whitespace between `{` and the first entry: covers
        // the `!prev_is_ws` branch where the comma+whitespace sit
        // forward of the removed entry.
        let p = pattern_from("{self, nixpkgs}: {}");
        let new_p = remove_output_arg(&p, "self").expect("entry must be found");
        assert_eq!(new_p.to_string(), "{nixpkgs}");
    }

    #[test]
    fn remove_output_arg_removes_first_entry_from_spaced_pattern() {
        // Spaced first entry in trailing-comma style: covers the
        // first-entry branch where `next_is_ws` is false and only the
        // lone trailing comma needs to go.
        let p = pattern_from("{ self, nixpkgs }: {}");
        let new_p = remove_output_arg(&p, "self").expect("entry must be found");
        assert_eq!(new_p.to_string(), "{ nixpkgs }");
    }

    #[test]
    fn remove_output_arg_removes_first_entry_from_leading_comma_style() {
        // First entry in leading-comma style: covers the branch that
        // strips the next entry's `\n  ,` separator and re-inserts a
        // single space so the result still parses.
        let p = pattern_from("{ self\n, nixpkgs\n, flake-utils\n}: {}");
        let new_p = remove_output_arg(&p, "self").expect("entry must be found");
        assert_eq!(new_p.to_string(), "{ nixpkgs\n, flake-utils\n}");
    }

    #[rstest::rstest]
    #[case::leading_comment_before(
        "{ self\n, nixpkgs\n, # pinned elsewhere\n  home-manager\n, nur\n}: {}",
        "{ self\n, nixpkgs\n, # pinned elsewhere\n  nur\n}"
    )]
    #[case::leading_comment_before_last(
        "{ self\n, nixpkgs\n, # pinned elsewhere\n  home-manager\n}: {}",
        "{ self\n, nixpkgs # pinned elsewhere\n}"
    )]
    #[case::leading_first_comment_after(
        "{ home-manager # pinned elsewhere\n, nur\n}: {}",
        "{ # pinned elsewhere\nnur\n}"
    )]
    #[case::trailing_comment_before_last(
        "{\n  nixpkgs, # pinned elsewhere\n  home-manager\n}: {}",
        "{\n  nixpkgs # pinned elsewhere\n}"
    )]
    #[case::trailing_comment_before_ellipsis(
        "{ self, nixpkgs,\n  #nixos-cosmic,\n  home-manager, ... }: {}",
        "{ self, nixpkgs,\n  #nixos-cosmic,\n  ... }"
    )]
    #[case::trailing_comment_own_line_before_last(
        "{\n  nixpkgs,\n  # pinned elsewhere\n  home-manager\n}: {}",
        "{\n  nixpkgs\n  # pinned elsewhere\n}"
    )]
    fn remove_output_arg_keeps_adjacent_comments(#[case] src: &str, #[case] expected: &str) {
        let p = pattern_from(src);
        let new_p = remove_output_arg(&p, "home-manager").expect("entry must be found");
        assert_eq!(new_p.to_string(), expected);
    }
}
