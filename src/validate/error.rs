//! [`ValidationError`], [`Severity`], and [`Location`].
//!
//! [`Severity`] separates fatal errors from non-fatal warnings.

use std::fmt;

/// 1-indexed line/column for error reporting.
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

/// A duplicate attribute and where its two definitions sit.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("duplicate attribute '{path}' at {duplicate} (first defined at {first})")]
pub struct DuplicateAttr {
    /// Attribute path, e.g. `a.b.c` or `inputs.nixpkgs.url`.
    pub path: String,
    /// Location of the first occurrence.
    pub first: Location,
    /// Location of the duplicate occurrence.
    pub duplicate: Location,
}

/// Severity classification for [`ValidationError`].
///
/// Errors abort the edit; warnings do not. Both flavours land in
/// [`ValidationResult::errors`] and [`ValidationResult::warnings`]
/// respectively, populated by [`super::validate_full`] and
/// [`super::validate_speculative`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Severity {
    Warning,
    Error,
}

/// Errors raised while parsing or analysing a flake.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ValidationError {
    /// rnix could not parse the source.
    #[error("parse error at {location}: {message}")]
    ParseError { message: String, location: Location },
    /// Duplicate attribute in an attribute set.
    #[error(transparent)]
    DuplicateAttribute(DuplicateAttr),
    /// A declared `inputs.X.inputs.Y.follows` chain forms a cycle.
    #[error("follows cycle at {location}: {}", format_edges(&cycle.edges))]
    FollowsCycle {
        cycle: crate::follows::Cycle,
        location: Location,
    },
    /// A follows declaration in `flake.nix` points at a nested input that no
    /// longer exists in `flake.lock`. Warning: the auto-follow pass should
    /// drop the declaration on the next run.
    #[error("stale follows at {location}: {} -> {} (source no longer present in flake.lock)", edge.source, edge.follows)]
    FollowsStale {
        /// Declared edge whose source has dropped out of `flake.lock`'s
        /// nested-input universe.
        edge: crate::follows::Edge,
        location: Location,
    },
    /// A follows target points at something that is not a top-level input,
    /// e.g. `inputs.foo.inputs.bar.follows = "does-not-exist"`.
    #[error("follows target not a top-level input at {location}: {} -> {}", edge.source, edge.follows)]
    FollowsTargetNotToplevel {
        edge: crate::follows::Edge,
        location: Location,
    },
    /// Two follows declarations share a source path but disagree on the
    /// target.
    #[error("contradicting follows at {location}: {}", format_edges(edges))]
    FollowsContradiction {
        edges: Vec<crate::follows::Edge>,
        location: Location,
    },
    /// A declared follows whose target diverges from the lockfile's
    /// resolution of the same source path. Warning: the user edited
    /// `flake.nix` but never ran `nix flake lock`.
    #[error(
        "stale-lock follows at {location}: {source_path} -> {declared_target} (flake.lock resolves to {}; run `nix flake lock`)",
        format_lock_target(lock_target)
    )]
    FollowsStaleLock {
        /// Source path of the declared follows, e.g. `crane.nixpkgs`.
        ///
        /// Renamed from `source` because thiserror treats a field named
        /// `source` as the error's `#[source]`.
        source_path: crate::follows::AttrPath,
        /// Target the declaration in `flake.nix` asks for.
        declared_target: crate::follows::AttrPath,
        /// Target the lockfile resolves the same source to. `None` if the
        /// lockfile has the path but no follows attached.
        lock_target: Option<crate::follows::AttrPath>,
        location: Location,
    },
    /// A follows path is deeper than the configured graph traversal bound.
    #[error("follows depth exceeded at {location}: {} -> {} reached depth {depth} (max {max_depth})", edge.source, edge.follows)]
    FollowsDepthExceeded {
        edge: crate::follows::Edge,
        depth: usize,
        max_depth: usize,
        location: Location,
    },
}

fn format_edges(edges: &[crate::follows::Edge]) -> String {
    edges
        .iter()
        .map(|e| format!("{} -> {}", e.source, e.follows))
        .collect::<Vec<_>>()
        .join("; ")
}

fn format_lock_target(target: &Option<crate::follows::AttrPath>) -> String {
    match target {
        Some(t) => t.to_string(),
        None => "<none>".to_string(),
    }
}

impl ValidationError {
    /// Severity for this variant.
    pub fn severity(&self) -> Severity {
        match self {
            ValidationError::FollowsStale { .. } | ValidationError::FollowsStaleLock { .. } => {
                Severity::Warning
            }
            _ => Severity::Error,
        }
    }
}

/// Errors and warnings collected during a single validation pass.
#[derive(Debug, Default)]
pub struct ValidationResult {
    pub errors: Vec<ValidationError>,
    pub warnings: Vec<ValidationError>,
}

impl ValidationResult {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::follows::{AttrPath, Cycle, Edge, EdgeOrigin};
    use crate::input::Range;

    fn declared_edge(source: &str, follows: &str) -> Edge {
        Edge {
            source: AttrPath::parse(source).unwrap(),
            follows: AttrPath::parse(follows).unwrap(),
            origin: EdgeOrigin::Declared {
                range: Range { start: 0, end: 0 },
            },
        }
    }

    fn loc() -> Location {
        Location { line: 1, column: 1 }
    }

    #[test]
    fn severity_classification() {
        let cases: Vec<(ValidationError, Severity)> = vec![
            (
                ValidationError::ParseError {
                    message: "x".into(),
                    location: loc(),
                },
                Severity::Error,
            ),
            (
                ValidationError::DuplicateAttribute(DuplicateAttr {
                    path: "a".into(),
                    first: loc(),
                    duplicate: loc(),
                }),
                Severity::Error,
            ),
            (
                ValidationError::FollowsCycle {
                    cycle: Cycle {
                        edges: vec![declared_edge("a", "a")],
                    },
                    location: loc(),
                },
                Severity::Error,
            ),
            (
                ValidationError::FollowsStale {
                    edge: declared_edge("a.b", "c"),
                    location: loc(),
                },
                Severity::Warning,
            ),
            (
                ValidationError::FollowsTargetNotToplevel {
                    edge: declared_edge("a.b", "missing"),
                    location: loc(),
                },
                Severity::Error,
            ),
            (
                ValidationError::FollowsContradiction {
                    edges: vec![declared_edge("a.b", "x"), declared_edge("a.b", "y")],
                    location: loc(),
                },
                Severity::Error,
            ),
            (
                ValidationError::FollowsStaleLock {
                    source_path: AttrPath::parse("a.b").unwrap(),
                    declared_target: AttrPath::parse("x").unwrap(),
                    lock_target: Some(AttrPath::parse("y").unwrap()),
                    location: loc(),
                },
                Severity::Warning,
            ),
            (
                ValidationError::FollowsDepthExceeded {
                    edge: declared_edge("a.b", "x"),
                    depth: 5,
                    max_depth: 4,
                    location: loc(),
                },
                Severity::Error,
            ),
        ];
        for (err, want) in cases {
            assert_eq!(err.severity(), want, "unexpected severity for {err:?}");
        }
    }
}
