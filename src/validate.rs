//! Validation for Nix flake expressions.
//!
//! - [`error`]: [`ValidationError`], [`Severity`], [`Location`].
//! - `syntax` (private): rnix parse errors and duplicate-attribute detection.
//! - `follows` (crate-private): cycle, stale, target, contradiction, and depth
//!   lints.
//!
//! [`validate`] runs the syntax-level lints. [`validate_full`] adds the
//! follows-graph lints that need a parsed [`InputMap`] and an optional
//! [`FlakeLock`].

pub mod error;
pub(crate) mod follows;
mod syntax;

pub use error::{DuplicateAttr, Location, Severity, ValidationError, ValidationResult};

pub(crate) use syntax::ParsedSource;

use crate::edit::InputMap;
use crate::follows::{DEFAULT_MAX_DEPTH, FollowsGraph};
use crate::lock::{FlakeLock, NestedInput};

/// Run the syntax-level lints over `source`: parse errors, duplicate
/// attributes, and the always-on declared-cycle check.
pub fn validate(source: &str) -> ValidationResult {
    let parsed = ParsedSource::new(source);
    validate_parsed(&parsed)
}

/// [`validate`] for callers that already hold a [`ParsedSource`], so the
/// rnix parse is shared with [`crate::walk::Walker`] construction.
pub(crate) fn validate_parsed(parsed: &ParsedSource) -> ValidationResult {
    let mut errors: Vec<ValidationError> = Vec::new();
    syntax::collect_with_parsed(parsed, &mut errors);
    if errors.is_empty() {
        let mut walker = crate::walk::Walker::from_root(parsed.syntax.clone());
        if walker.walk(&crate::change::Change::None).is_ok() {
            let graph = crate::follows::FollowsGraph::from_declared(&walker.inputs);
            let offset_to_location = |offset: usize| parsed.line_map.offset_to_location(offset);
            errors.extend(follows::lint_follows_cycle(&graph, &offset_to_location));
        }
    }
    ValidationResult {
        errors,
        warnings: Vec::new(),
    }
}

/// Run syntax checks plus every follows-graph lint.
///
/// Walks `flake.lock` once via [`FlakeLock::nested_inputs`], builds the lock
/// graph from that single walk, and hands both to
/// [`validate_full_with_lock_graph`].
pub fn validate_full(
    source: &str,
    inputs: &InputMap,
    lock: Option<&FlakeLock>,
) -> ValidationResult {
    let parsed = ParsedSource::new(source);
    let nested_inputs = lock.map(FlakeLock::nested_inputs);
    let lock_graph = nested_inputs
        .as_deref()
        .map(FollowsGraph::from_nested_inputs);
    validate_full_with_lock_graph(
        &parsed,
        inputs,
        lock_graph.as_ref(),
        nested_inputs.as_deref().unwrap_or(&[]),
    )
}

/// Like [`validate_full`] but skips the lock-drift lints (`lint_follows_stale`
/// and `lint_follows_stale_lock`) that compare declared edges in `flake.nix`
/// against `flake.lock`.
///
/// For speculative validation during a multi-step apply. The lockfile cannot
/// reflect mid-batch text edits, so a freshly-added follows always looks
/// stale relative to the on-disk lock. Running lock-drift lints there would
/// flag every in-progress edit as drift.
pub fn validate_speculative(
    source: &str,
    inputs: &InputMap,
    lock: Option<&FlakeLock>,
) -> ValidationResult {
    let parsed = ParsedSource::new(source);
    let lock_graph = lock.map(FollowsGraph::from_lock);
    validate_speculative_parsed(&parsed, inputs, lock_graph.as_ref())
}

/// [`validate_speculative`] for callers that already hold a [`ParsedSource`]
/// and a pre-built [`FollowsGraph`] of the lockfile (typically from
/// [`FollowsGraph::from_lock`]). Pass `lock_graph = None` to validate against
/// declared edges only.
///
/// Skips the duplicate-attribute lints. The apply loop's
/// [`crate::change::Change`] variants cannot introduce duplicates the source
/// did not already carry: `Add` rejects existing ids, `Follows` mutates in
/// place, `Remove` only deletes. [`validate_full`] runs once before the batch
/// and covers the original duplicate state.
pub(crate) fn validate_speculative_parsed(
    parsed: &ParsedSource,
    inputs: &InputMap,
    lock_graph: Option<&FollowsGraph>,
) -> ValidationResult {
    let mut errors: Vec<ValidationError> = parsed.parse_errors.to_vec();
    let mut warnings: Vec<ValidationError> = Vec::new();
    let graph = follows::build_graph_with_lock_graph(inputs, lock_graph, DEFAULT_MAX_DEPTH);
    run_follows_lints(parsed, inputs, &graph, None, &mut errors, &mut warnings);
    ValidationResult { errors, warnings }
}

/// [`validate_full`] for callers that already hold a [`ParsedSource`], a
/// lockfile-derived [`FollowsGraph`] (from [`FollowsGraph::from_lock`] or
/// [`FollowsGraph::from_nested_inputs`]), and the lockfile's nested-input
/// set. Reuses all three instead of re-walking `flake.lock`.
///
/// `lock_graph = Some(..)` enables the lock-drift lints (`lint_follows_stale`
/// and `lint_follows_stale_lock`), which read `nested_inputs`. `None` skips
/// them and `nested_inputs` is ignored.
pub(crate) fn validate_full_with_lock_graph(
    parsed: &ParsedSource,
    inputs: &InputMap,
    lock_graph: Option<&FollowsGraph>,
    nested_inputs: &[NestedInput],
) -> ValidationResult {
    let mut errors: Vec<ValidationError> = Vec::new();
    let mut warnings: Vec<ValidationError> = Vec::new();
    syntax::collect_with_parsed(parsed, &mut errors);
    let graph = follows::build_graph_with_lock_graph(inputs, lock_graph, DEFAULT_MAX_DEPTH);
    let nested = lock_graph.is_some().then_some(nested_inputs);
    run_follows_lints(parsed, inputs, &graph, nested, &mut errors, &mut warnings);
    ValidationResult { errors, warnings }
}

/// Run every follows-graph lint and route results into `errors`/`warnings` by
/// severity. `nested_inputs` enables the lock-drift lints (stale and
/// stale-lock); pass `None` to skip them.
fn run_follows_lints(
    parsed: &ParsedSource,
    inputs: &InputMap,
    graph: &FollowsGraph,
    nested_inputs: Option<&[NestedInput]>,
    errors: &mut Vec<ValidationError>,
    warnings: &mut Vec<ValidationError>,
) {
    let offset_to_location = |offset: usize| parsed.line_map.offset_to_location(offset);

    let mut candidates: Vec<ValidationError> = Vec::new();
    candidates.extend(follows::lint_follows_cycle(graph, &offset_to_location));
    if let Some(nested) = nested_inputs {
        candidates.extend(follows::lint_follows_stale(graph, &offset_to_location));
        candidates.extend(follows::lint_follows_stale_lock(
            graph,
            nested,
            &offset_to_location,
        ));
    }
    let top_level = follows::top_level_names(inputs);
    candidates.extend(follows::lint_follows_target_not_toplevel(
        graph,
        &top_level,
        &offset_to_location,
    ));
    candidates.extend(follows::lint_follows_contradiction(
        graph,
        &offset_to_location,
    ));
    candidates.extend(follows::lint_follows_depth_exceeded(
        graph,
        DEFAULT_MAX_DEPTH,
        &offset_to_location,
    ));

    for err in candidates {
        match err.severity() {
            Severity::Warning => warnings.push(err),
            Severity::Error => errors.push(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::InputMap;
    use crate::follows::{AttrPath, Segment};
    use crate::input::{Follows, Input, Range};
    use crate::validate::error::DuplicateAttr;

    fn expect_duplicate(err: &ValidationError) -> &DuplicateAttr {
        match err {
            ValidationError::DuplicateAttribute(dup) => dup,
            other => panic!("expected DuplicateAttribute, got {other:?}"),
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

    #[test]
    fn mergeable_attrsets_valid() {
        let source = r#"{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs";
  };
  inputs = {
    flake-utils.url = "github:numtide/flake-utils";
  };
}"#;
        let result = validate(source);
        assert!(
            result.is_ok(),
            "expected no errors, got: {:?}",
            result.errors
        );
    }

    #[test]
    fn mergeable_attrsets_with_comments() {
        // autofirma-nix pattern: comment-separated input groups
        let source = r#"{
  # Common inputs
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    home-manager.url = "github:nix-community/home-manager";
  };

  # Autofirma sources
  inputs = {
    jmulticard-src = {
      url = "github:ctt-gob-es/jmulticard/v2.0";
      flake = false;
    };
  };

  outputs = { self, nixpkgs, ... }: { };
}"#;
        let result = validate(source);
        assert!(
            result.is_ok(),
            "expected no errors, got: {:?}",
            result.errors
        );
    }

    #[test]
    fn mergeable_attrsets_cross_duplicate() {
        let source = r#"{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs";
  };
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/unstable";
  };
}"#;
        let result = validate(source);
        assert!(result.has_errors());
        assert_eq!(result.errors.len(), 1);

        let dup = expect_duplicate(&result.errors[0]);
        assert_eq!(dup.path, "nixpkgs.url");
    }

    #[test]
    fn non_attrset_duplicate_still_errors() {
        let source = r#"{ a = { x = 1; }; a = 2; }"#;
        let result = validate(source);
        assert!(result.has_errors());
        assert_eq!(result.errors.len(), 1);

        let dup = expect_duplicate(&result.errors[0]);
        assert_eq!(dup.path, "a");
    }

    #[test]
    fn follows_cycle_self_edge_lints() {
        let source = r#"{
  inputs.foo = {
    url = "github:owner/foo";
    inputs.foo.follows = "foo/foo";
  };
  outputs = { ... }: { };
}"#;
        let result = validate(source);
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e, ValidationError::FollowsCycle { .. })),
            "expected FollowsCycle, got: {:?}",
            result.errors,
        );
    }

    #[test]
    fn three_mergeable_attrsets() {
        let source = r#"{
  inputs = { a.url = "a"; };
  inputs = { b.url = "b"; };
  inputs = { c.url = "c"; };
}"#;
        let result = validate(source);
        assert!(
            result.is_ok(),
            "expected no errors, got: {:?}",
            result.errors
        );
    }

    fn seg(s: &str) -> Segment {
        Segment::from_unquoted(s).unwrap()
    }

    fn path(s: &str) -> AttrPath {
        AttrPath::parse(s).unwrap()
    }

    fn declared_input(id: &str, follows: &[(&str, &str)]) -> Input {
        let mut input = Input::new(seg(id));
        for (parent, target) in follows {
            input.follows.push(Follows::Indirect {
                path: AttrPath::new(seg(parent)),
                target: Some(path(target)),
            });
        }
        input.range = Range { start: 1, end: 2 };
        input
    }

    fn make_inputs(items: Vec<Input>) -> InputMap {
        let mut map = InputMap::new();
        for input in items {
            map.insert(input.id().as_str().to_string(), input);
        }
        map
    }

    #[test]
    fn validate_full_emits_target_not_toplevel_by_default() {
        let inputs = make_inputs(vec![declared_input("a", &[("b", "missing")])]);
        let result = validate_full("{}", &inputs, None);
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e, ValidationError::FollowsTargetNotToplevel { .. })),
            "expected target-not-toplevel error, got: {:?}",
            result.errors,
        );
    }

    #[test]
    fn validate_full_separates_warnings_from_errors() {
        // Stale follows is a warning, target-not-toplevel is an error, and
        // both can fire on the same input.
        let inputs = make_inputs(vec![declared_input("a", &[("b", "missing")])]);
        let lock_text = r#"{
  "nodes": {
    "a": {
      "locked": { "lastModified": 1, "narHash": "", "owner": "o", "repo": "r", "rev": "abc", "type": "github" },
      "original": { "owner": "o", "repo": "r", "type": "github" }
    },
    "root": { "inputs": { "a": "a" } }
  },
  "root": "root",
  "version": 7
}"#;
        let lock = FlakeLock::read_from_str(lock_text).unwrap();
        let result = validate_full("{}", &inputs, Some(&lock));
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e, ValidationError::FollowsTargetNotToplevel { .. })),
        );
        assert!(
            result
                .warnings
                .iter()
                .any(|e| matches!(e, ValidationError::FollowsStale { .. })),
            "expected at least one stale warning, got warnings: {:?}",
            result.warnings,
        );
    }
}
