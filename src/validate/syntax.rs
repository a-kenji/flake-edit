//! Syntax-level lints: rnix parse errors and duplicate-attribute detection.
//!
//! Entry point is [`collect`]. [`LineMap`] resolves CST byte offsets into
//! [`Location`]s.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use rnix::{Root, SyntaxKind, SyntaxNode, TextRange};

use super::error::{DuplicateAttr, Location, ValidationError};

/// Lookup table that maps CST byte offsets to 1-indexed [`Location`]s.
pub(super) struct LineMap {
    line_starts: Vec<usize>,
}

impl LineMap {
    pub(super) fn new(source: &str) -> Self {
        let mut starts = vec![0];
        for (i, c) in source.char_indices() {
            if c == '\n' {
                starts.push(i + 1);
            }
        }
        Self {
            line_starts: starts,
        }
    }

    pub(super) fn offset_to_location(&self, offset: usize) -> Location {
        let line = self
            .line_starts
            .iter()
            .rposition(|&start| start <= offset)
            .unwrap_or(0);
        let column = offset - self.line_starts[line];
        Location {
            line: line + 1,
            column: column + 1,
        }
    }

    pub(super) fn range_to_location(&self, range: TextRange) -> Location {
        self.offset_to_location(range.start().into())
    }

    pub(super) fn fallback_eof(&self) -> Location {
        Location {
            line: self.line_starts.len(),
            column: 1,
        }
    }
}

/// Parse `source` with rnix and append parse errors and duplicate-attribute
/// findings to `errors`.
pub(super) fn collect(source: &str, errors: &mut Vec<ValidationError>) {
    let line_map = LineMap::new(source);
    let root = Root::parse(source);

    for error in root.errors() {
        let location = parse_error_location(error, &line_map);
        errors.push(ValidationError::ParseError {
            message: error.to_string(),
            location,
        });
    }

    let syntax = root.syntax();
    check_node(&syntax, &line_map, errors);
}

fn parse_error_location(error: &rnix::ParseError, line_map: &LineMap) -> Location {
    use rnix::ParseError::*;
    match error {
        Unexpected(r)
        | UnexpectedExtra(r)
        | UnexpectedWanted(_, r, _)
        | UnexpectedDoubleBind(r)
        | DuplicatedArgs(r, _) => line_map.range_to_location(*r),
        UnexpectedEOF | UnexpectedEOFWanted(_) | RecursionLimitExceeded | _ => {
            line_map.fallback_eof()
        }
    }
}

/// Render an attribute path node as a dotted string, e.g. `a.b.c`.
fn extract_attrpath(attrpath: &SyntaxNode) -> String {
    attrpath
        .children()
        .map(|child| match crate::follows::Segment::from_syntax(&child) {
            Ok(seg) => seg.into_string(),
            Err(_) => child.to_string(),
        })
        .collect::<Vec<_>>()
        .join(".")
}

fn value_is_attrset(node: &SyntaxNode) -> bool {
    node.children()
        .any(|c| c.kind() == SyntaxKind::NODE_ATTR_SET)
}

fn check_node(node: &SyntaxNode, line_map: &LineMap, errors: &mut Vec<ValidationError>) {
    if node.kind() == SyntaxKind::NODE_ATTR_SET {
        check_attr_set(node, line_map, errors);
    }
    for child in node.children() {
        check_node(&child, line_map, errors);
    }
}

/// Check an attribute set for duplicate attributes.
///
/// Nix merges duplicate attribute names whose values are both attribute
/// sets, e.g.
/// ```nix
/// {
///   inputs = { nixpkgs.url = "..."; };
///   inputs = { flake-utils.url = "..."; };
/// }
/// ```
/// is equivalent to a single `inputs` carrying both entries. The merge is
/// allowed. The merged contents are still checked for true conflicts.
fn check_attr_set(attr_set: &SyntaxNode, line_map: &LineMap, errors: &mut Vec<ValidationError>) {
    let mut seen: HashMap<String, (Location, bool, SyntaxNode)> = HashMap::new();
    let mut merged_attrsets: HashMap<String, Vec<SyntaxNode>> = HashMap::new();

    for child in attr_set.children() {
        if child.kind() == SyntaxKind::NODE_ATTRPATH_VALUE
            && let Some(attrpath) = child
                .children()
                .find(|c| c.kind() == SyntaxKind::NODE_ATTRPATH)
        {
            let path = extract_attrpath(&attrpath);
            let location = line_map.range_to_location(attrpath.text_range());
            let is_attrset = value_is_attrset(&child);

            match seen.entry(path.clone()) {
                Entry::Occupied(entry) => {
                    let (ref first_loc, first_is_attrset, _) = *entry.get();
                    if first_is_attrset && is_attrset {
                        merged_attrsets.entry(path).or_default().push(child.clone());
                    } else {
                        errors.push(ValidationError::DuplicateAttribute(DuplicateAttr {
                            path: entry.key().clone(),
                            first: first_loc.clone(),
                            duplicate: location,
                        }));
                    }
                }
                Entry::Vacant(entry) => {
                    if is_attrset {
                        merged_attrsets.entry(path).or_default().push(child.clone());
                    }
                    entry.insert((location, is_attrset, child.clone()));
                }
            }
        }
    }

    for nodes in merged_attrsets.values() {
        if nodes.len() < 2 {
            continue;
        }
        let mut cross_seen: HashMap<String, Location> = HashMap::new();
        for node in nodes {
            for attrset_child in node.children() {
                if attrset_child.kind() != SyntaxKind::NODE_ATTR_SET {
                    continue;
                }
                for inner in attrset_child.children() {
                    if inner.kind() == SyntaxKind::NODE_ATTRPATH_VALUE
                        && let Some(inner_path_node) = inner
                            .children()
                            .find(|c| c.kind() == SyntaxKind::NODE_ATTRPATH)
                    {
                        let inner_path = extract_attrpath(&inner_path_node);
                        let inner_loc = line_map.range_to_location(inner_path_node.text_range());

                        match cross_seen.entry(inner_path) {
                            Entry::Occupied(e) => {
                                errors.push(ValidationError::DuplicateAttribute(DuplicateAttr {
                                    path: e.key().clone(),
                                    first: e.get().clone(),
                                    duplicate: inner_loc,
                                }));
                            }
                            Entry::Vacant(e) => {
                                e.insert(inner_loc);
                            }
                        }
                    }
                }
            }
        }
    }
}
