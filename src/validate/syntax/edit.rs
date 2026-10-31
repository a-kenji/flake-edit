//! Validation of an edited tree without a full reparse of its text.

use rnix::{Root, SyntaxElement, SyntaxKind, SyntaxNode};
use rowan::NodeOrToken;

use super::{LineMap, ParsedSource, push_line_starts};
use crate::walk::Splice;

impl ParsedSource {
    pub(crate) fn from_edit(edited: SyntaxNode, splice: Option<&Splice>) -> Self {
        if !splice.is_none_or(parses_in_place) {
            return Self::new(&edited.to_string());
        }
        // A splice that changes lexing beyond one neighbour breaks the local reparse.
        debug_assert_eq!(
            *edited.green(),
            *Root::parse(&edited.to_string()).syntax().green()
        );
        Self {
            line_map: LineMap::from_syntax(&edited),
            parse_errors: Vec::new(),
            syntax: edited,
        }
    }
}

impl LineMap {
    fn from_syntax(node: &SyntaxNode) -> Self {
        let mut line_starts = vec![0];
        let mut offset = 0;
        node.text().for_each_chunk(|chunk| {
            push_line_starts(&mut line_starts, offset, chunk);
            offset += chunk.len();
        });
        Self { line_starts }
    }
}

fn parses_in_place(splice: &Splice) -> bool {
    let Some((parent, children)) = splice.in_attr_set() else {
        return false;
    };
    let elements: Vec<SyntaxElement> = parent.children_with_tokens().collect();
    let open = elements
        .iter()
        .position(|e| e.kind() == SyntaxKind::TOKEN_L_BRACE);
    let close = elements
        .iter()
        .rposition(|e| e.kind() == SyntaxKind::TOKEN_R_BRACE);
    let (Some(open), Some(close)) = (open, close) else {
        return false;
    };
    if children.start <= open || children.end > close {
        return false;
    }
    let body = &elements[(children.start - 1).max(open + 1)..(children.end + 1).min(close)];

    let mut text = String::from("{");
    for element in body {
        text.push_str(&element.to_string());
    }
    text.push('}');
    let reparsed = Root::parse(&text);
    if !reparsed.errors().is_empty() {
        return false;
    }
    let Some(set) = reparsed.syntax().first_child() else {
        return false;
    };
    let reparsed: Vec<SyntaxElement> = set.children_with_tokens().collect();
    set.kind() == SyntaxKind::NODE_ATTR_SET
        && reparsed.len() == body.len() + 2
        && reparsed[1..=body.len()]
            .iter()
            .zip(body)
            .all(|(r, b)| equal_green(r, b))
}

fn equal_green(a: &SyntaxElement, b: &SyntaxElement) -> bool {
    match (a, b) {
        (NodeOrToken::Node(a), NodeOrToken::Node(b)) => *a.green() == *b.green(),
        (NodeOrToken::Token(a), NodeOrToken::Token(b)) => a.green() == b.green(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::change::{Change, ChangeId};
    use crate::edit::FlakeEdit;
    use crate::follows::AttrPath;

    fn parse(text: &str) -> SyntaxNode {
        let parsed = ParsedSource::new(text);
        assert!(parsed.parse_errors.is_empty(), "{:?}", parsed.parse_errors);
        parsed.syntax
    }

    fn edit(before: &SyntaxNode, change: Change) -> SyntaxNode {
        FlakeEdit::from_syntax(before.clone())
            .apply_change_tree(change)
            .expect("change applies")
            .expect("change edits the tree")
    }

    fn from_edit(before: &SyntaxNode, edited: SyntaxNode) -> ParsedSource {
        let splice = Splice::find(before, &edited);
        ParsedSource::from_edit(edited, splice.as_ref())
    }

    fn follows(input: &str, target: &str) -> Change {
        Change::Follows {
            input: ChangeId::parse(input).unwrap(),
            target: AttrPath::parse(target).unwrap(),
        }
    }

    fn toggle(id: &str, uri: &str, previous: &str) -> Change {
        Change::Toggle {
            id: ChangeId::parse(id).unwrap(),
            uri: uri.to_owned(),
            previous: previous.to_owned(),
        }
    }

    #[test]
    fn follows_edit_keeps_the_edited_tree() {
        for (before, input) in [
            (
                r#"{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    home-manager = {
      url = "github:nix-community/home-manager";
    };
  };
  outputs = _: { };
}
"#,
                "home-manager.nixpkgs",
            ),
            (
                r#"{
  inputs = {
    flake-edit.url = "github:a-kenji/flake-edit";
    mprisd.url = "github:a-kenji/mprisd";
    nixpkgs.follows = "mprisd/nixpkgs";
  };
  outputs = _: { };
}
"#,
                "flake-edit.nixpkgs",
            ),
        ] {
            let before = parse(before);
            let edited = edit(&before, follows(input, "nixpkgs"));

            let parsed = from_edit(&before, edited.clone());
            assert_eq!(parsed.parse_errors, []);
            assert_eq!(parsed.syntax, edited);
            assert_eq!(
                *edited.green(),
                *Root::parse(&edited.to_string()).syntax().green()
            );
        }
    }

    #[test]
    fn unparseable_edit_reports_parse_errors() {
        for before in [
            r#"{
  inputs = {
    rust-overlay = {
      url =
        "github:oxalica/rust-overlay";
    };
  };
  outputs = _: { };
}
"#,
            r#"{
  inputs = {
    rust-overlay.url =
      "github:oxalica/rust-overlay";
  };
  outputs = _: { };
}
"#,
        ] {
            let before = parse(before);
            let edited = edit(
                &before,
                toggle(
                    "rust-overlay",
                    "github:a-kenji/rust-overlay",
                    "github:oxalica/rust-overlay",
                ),
            );
            let full = ParsedSource::new(&edited.to_string());
            assert!(
                !full.parse_errors.is_empty(),
                "the edited text must not parse"
            );

            let parsed = from_edit(&before, edited);
            assert_eq!(parsed.parse_errors, full.parse_errors);
        }
    }

    #[test]
    fn line_map_from_syntax_matches_text() {
        let text =
            "{\n  inputs.nixpkgs.url = \"github:NixOS/nixpkgs\";\n\n  outputs = _: { };\n}\n";
        assert_eq!(
            LineMap::from_syntax(&parse(text)).line_starts,
            LineMap::new(text).line_starts,
        );
    }
}
