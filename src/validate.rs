//! Validation for Nix expressions.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::fmt;

use rnix::{Root, SyntaxKind, SyntaxNode, TextRange};

/// Location information for error reporting (1-indexed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub line: usize,
    pub column: usize,
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}, column {}", self.line, self.column)
    }
}

/// Information about a duplicate attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateAttr {
    /// The attribute path, e.g., "a.b.c" or "inputs.nixpkgs.url".
    pub path: String,
    /// Location of the first occurrence.
    pub first: Location,
    /// Location of the duplicate occurrence.
    pub duplicate: Location,
}

impl fmt::Display for DuplicateAttr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "duplicate attribute '{}' at {} (first defined at {})",
            self.path, self.duplicate, self.first
        )
    }
}

/// Validation errors that can occur when parsing Nix expressions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    /// Syntax error from rnix parser.
    ParseError { message: String, location: Location },
    /// Duplicate attribute in an attribute set.
    DuplicateAttribute(DuplicateAttr),
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValidationError::ParseError { message, location } => {
                write!(f, "parse error at {}: {}", location, message)
            }
            ValidationError::DuplicateAttribute(dup) => write!(f, "{}", dup),
        }
    }
}

/// Result of validation containing any errors found.
#[derive(Debug, Default)]
pub struct ValidationResult {
    pub errors: Vec<ValidationError>,
}

impl ValidationResult {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }
}

/// Validator for Nix expressions.
pub struct Validator {
    source: String,
    /// Byte offsets where each line starts (for computing line/column).
    line_starts: Vec<usize>,
}

/// Extract the full attribute path as a string, e.g., "a.b.c".
fn extract_attrpath(attrpath: &SyntaxNode) -> String {
    attrpath
        .children()
        .map(|child| {
            let s = child.to_string();
            // Unquote string attribute names: `"a"` -> `a`
            if child.kind() == SyntaxKind::NODE_STRING {
                s.trim_matches('"').to_string()
            } else {
                s
            }
        })
        .collect::<Vec<_>>()
        .join(".")
}

impl Validator {
    /// Create a new validator for the given source.
    pub fn new(source: &str) -> Self {
        let line_starts = Self::compute_line_starts(source);
        Self {
            source: source.to_string(),
            line_starts,
        }
    }

    /// Compute byte offsets for the start of each line.
    fn compute_line_starts(source: &str) -> Vec<usize> {
        let mut starts = vec![0];
        for (i, c) in source.char_indices() {
            if c == '\n' {
                starts.push(i + 1);
            }
        }
        starts
    }

    /// Convert a TextRange to a Location (using the start position).
    fn range_to_location(&self, range: TextRange) -> Location {
        self.offset_to_location(range.start().into())
    }

    /// Convert a byte offset to line and column (1-indexed).
    fn offset_to_location(&self, offset: usize) -> Location {
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

    /// Validate the source and return any errors found.
    pub fn validate(&self) -> ValidationResult {
        let root = Root::parse(&self.source);
        let mut errors = Vec::new();

        // Collect rnix parse errors
        for error in root.errors() {
            let location = self.parse_error_location(error);
            errors.push(ValidationError::ParseError {
                message: error.to_string(),
                location,
            });
        }

        // Check for duplicate attributes
        let syntax = root.syntax();
        self.check_node(&syntax, &mut errors);

        ValidationResult { errors }
    }

    /// Extract location from an rnix ParseError.
    fn parse_error_location(&self, error: &rnix::ParseError) -> Location {
        use rnix::ParseError::*;
        match error {
            Unexpected(r)
            | UnexpectedExtra(r)
            | UnexpectedWanted(_, r, _)
            | UnexpectedDoubleBind(r)
            | DuplicatedArgs(r, _) => self.range_to_location(*r),
            UnexpectedEOF | UnexpectedEOFWanted(_) | RecursionLimitExceeded | _ => Location {
                line: self.line_starts.len(),
                column: 1,
            },
        }
    }

    /// Recursively check a node and its descendants for duplicate attributes.
    fn check_node(&self, node: &SyntaxNode, errors: &mut Vec<ValidationError>) {
        if node.kind() == SyntaxKind::NODE_ATTR_SET {
            self.check_attr_set(node, errors);
        }

        for child in node.children() {
            self.check_node(&child, errors);
        }
    }

    /// Check an attribute set for duplicate attributes.
    fn check_attr_set(&self, attr_set: &SyntaxNode, errors: &mut Vec<ValidationError>) {
        let mut seen: HashMap<String, Location> = HashMap::new();

        for child in attr_set.children() {
            if child.kind() == SyntaxKind::NODE_ATTRPATH_VALUE
                && let Some(attrpath) = child
                    .children()
                    .find(|c| c.kind() == SyntaxKind::NODE_ATTRPATH)
            {
                let path = extract_attrpath(&attrpath);
                let location = self.range_to_location(attrpath.text_range());

                match seen.entry(path) {
                    Entry::Occupied(entry) => {
                        errors.push(ValidationError::DuplicateAttribute(DuplicateAttr {
                            path: entry.key().clone(),
                            first: entry.get().clone(),
                            duplicate: location,
                        }));
                    }
                    Entry::Vacant(entry) => {
                        entry.insert(location);
                    }
                }
            }
        }
    }
}

/// Convenience function to validate source and return errors.
pub fn validate(source: &str) -> ValidationResult {
    Validator::new(source).validate()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expect_duplicate(err: &ValidationError) -> &DuplicateAttr {
        match err {
            ValidationError::DuplicateAttribute(dup) => dup,
            ValidationError::ParseError { .. } => {
                panic!("expected DuplicateAttribute, got ParseError")
            }
        }
    }

    #[test]
    fn simple_duplicate() {
        let source = "{ a = 1; a = 2; }";
        let result = validate(source);
        assert!(result.has_errors());
        assert_eq!(result.errors.len(), 1);

        let dup = expect_duplicate(&result.errors[0]);
        assert_eq!(dup.path, "a");
        assert_eq!(dup.first.line, 1);
        assert_eq!(dup.first.column, 3);
        assert_eq!(dup.duplicate.line, 1);
        assert_eq!(dup.duplicate.column, 10);
    }

    #[test]
    fn nested_path_duplicate() {
        let source = "{ a.b.c = 1; a.b.c = 2; }";
        let result = validate(source);
        assert!(result.has_errors());
        assert_eq!(result.errors.len(), 1);

        let dup = expect_duplicate(&result.errors[0]);
        assert_eq!(dup.path, "a.b.c");
    }

    #[test]
    fn different_paths_valid() {
        let source = "{ a.b = 1; a.c = 2; }";
        let result = validate(source);
        assert!(result.is_ok());
    }

    #[test]
    fn flake_style_duplicate() {
        let source = r#"{ inputs.nixpkgs.url = "github:nixos/nixpkgs"; inputs.nixpkgs.url = "github:nixos/nixpkgs/unstable"; }"#;
        let result = validate(source);
        assert!(result.has_errors());
        assert_eq!(result.errors.len(), 1);

        let dup = expect_duplicate(&result.errors[0]);
        assert_eq!(dup.path, "inputs.nixpkgs.url");
    }

    #[test]
    fn quoted_attribute_duplicate() {
        let source = r#"{ "a" = 1; a = 2; }"#;
        let result = validate(source);
        assert!(result.has_errors());
        assert_eq!(result.errors.len(), 1);

        let dup = expect_duplicate(&result.errors[0]);
        assert_eq!(dup.path, "a");
    }

    #[test]
    fn nested_attr_set_duplicate() {
        let source = "{ outer = { inner = 1; inner = 2; }; }";
        let result = validate(source);
        assert!(result.has_errors());
        assert_eq!(result.errors.len(), 1);

        let dup = expect_duplicate(&result.errors[0]);
        assert_eq!(dup.path, "inner");
    }

    #[test]
    fn multiple_duplicates() {
        let source = "{ a = 1; a = 2; b = 3; b = 4; }";
        let result = validate(source);
        assert!(result.has_errors());
        assert_eq!(result.errors.len(), 2);
    }

    #[test]
    fn multiline_flake() {
        let source = r#"{
  inputs.nixpkgs.url = "github:nixos/nixpkgs";
  inputs.nixpkgs.url = "github:nixos/nixpkgs/unstable";
  outputs = { ... }: { };
}"#;
        let result = validate(source);
        assert!(result.has_errors());
        assert_eq!(result.errors.len(), 1);

        let dup = expect_duplicate(&result.errors[0]);
        assert_eq!(dup.path, "inputs.nixpkgs.url");
        assert_eq!(dup.first.line, 2);
        assert_eq!(dup.duplicate.line, 3);
    }

    #[test]
    fn valid_flake() {
        let source = r#"{
  inputs.nixpkgs.url = "github:nixos/nixpkgs";
  inputs.flake-utils.url = "github:numtide/flake-utils";
  outputs = { self, nixpkgs, flake-utils }: { };
}"#;
        let result = validate(source);
        assert!(result.is_ok());
    }

    #[test]
    fn empty_attr_set() {
        let source = "{ }";
        let result = validate(source);
        assert!(result.is_ok());
    }

    #[test]
    fn single_attribute() {
        let source = "{ a = 1; }";
        let result = validate(source);
        assert!(result.is_ok());
    }

    #[test]
    fn parse_error_missing_semicolon() {
        let source = "{ a = 1 }";
        let result = validate(source);
        assert!(result.has_errors());
        assert!(matches!(
            &result.errors[0],
            ValidationError::ParseError { .. }
        ));
    }

    #[test]
    fn parse_error_unclosed_brace() {
        let source = "{ a = 1;";
        let result = validate(source);
        assert!(result.has_errors());
        assert!(matches!(
            &result.errors[0],
            ValidationError::ParseError { .. }
        ));
    }
}
