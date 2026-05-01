//! [`ValidationError`], [`Severity`], and [`Location`].
//!
//! Validation distinguishes errors (which abort the operation) from
//! warnings (which ride alongside a successful [`crate::edit::ApplyOutcome`])
//! through a typed [`Severity`].

use std::fmt;

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

/// Coarse severity classification for [`ValidationError`].
///
/// Errors abort the operation; warnings are collected and surfaced alongside
/// a successful [`crate::edit::ApplyOutcome`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Error,
}

/// Validation errors that can occur when parsing or analysing a flake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    /// Syntax error from rnix parser.
    ParseError { message: String, location: Location },
    /// Duplicate attribute in an attribute set.
    DuplicateAttribute(DuplicateAttr),
    /// A declared `inputs.X.inputs.Y.follows` chain forms a cycle.
    FollowsCycle {
        cycle: crate::follows::Cycle,
        location: Location,
    },
    /// A follows declaration in `flake.nix` references a nested input that no
    /// longer exists in `flake.lock`. Warning severity: the auto-follow pass
    /// should drop the declaration on the next run.
    FollowsStale {
        /// The declared edge whose source is no longer present in
        /// `flake.lock`'s nested-input universe.
        edge: crate::follows::Edge,
        location: Location,
    },
    /// A follows target references something that isn't a top-level input of
    /// the flake (e.g. `inputs.foo.inputs.bar.follows = "does-not-exist"`).
    FollowsTargetNotToplevel {
        edge: crate::follows::Edge,
        location: Location,
    },
    /// Two follows declarations contradict each other: same source path,
    /// different targets.
    FollowsContradiction {
        edges: Vec<crate::follows::Edge>,
        location: Location,
    },
    /// A declared follows whose target diverges from the lockfile's
    /// resolution for the same source path. Warning severity: the user
    /// edited `flake.nix` but never ran `nix flake lock`.
    FollowsStaleLock {
        /// Source path of the declared follows (e.g. `crane.nixpkgs`).
        source: crate::follows::AttrPath,
        /// Target the declaration in `flake.nix` asks for.
        declared_target: crate::follows::AttrPath,
        /// Target the lockfile resolves the same source to. `None` means the
        /// lockfile has the path but no follows attached at all.
        lock_target: Option<crate::follows::AttrPath>,
        location: Location,
    },
    /// A follows path's traversal exceeds the configured graph depth bound.
    FollowsDepthExceeded {
        edge: crate::follows::Edge,
        depth: usize,
        max_depth: usize,
        location: Location,
    },
}

impl ValidationError {
    /// Severity classification for this error.
    pub fn severity(&self) -> Severity {
        match self {
            ValidationError::FollowsStale { .. } | ValidationError::FollowsStaleLock { .. } => {
                Severity::Warning
            }
            _ => Severity::Error,
        }
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValidationError::ParseError { message, location } => {
                write!(f, "parse error at {}: {}", location, message)
            }
            ValidationError::DuplicateAttribute(dup) => write!(f, "{}", dup),
            ValidationError::FollowsCycle { cycle, location } => {
                let chain = cycle
                    .edges
                    .iter()
                    .map(|e| format!("{} -> {}", e.source, e.follows))
                    .collect::<Vec<_>>()
                    .join("; ");
                write!(f, "follows cycle at {}: {}", location, chain)
            }
            ValidationError::FollowsStale { edge, location } => {
                write!(
                    f,
                    "stale follows at {}: {} -> {} (source no longer present in flake.lock)",
                    location, edge.source, edge.follows
                )
            }
            ValidationError::FollowsTargetNotToplevel { edge, location } => {
                write!(
                    f,
                    "follows target not a top-level input at {}: {} -> {}",
                    location, edge.source, edge.follows
                )
            }
            ValidationError::FollowsContradiction { edges, location } => {
                let pairs = edges
                    .iter()
                    .map(|e| format!("{} -> {}", e.source, e.follows))
                    .collect::<Vec<_>>()
                    .join("; ");
                write!(f, "contradicting follows at {}: {}", location, pairs)
            }
            ValidationError::FollowsStaleLock {
                source,
                declared_target,
                lock_target,
                location,
            } => {
                let lock = match lock_target {
                    Some(t) => t.to_string(),
                    None => "<none>".to_string(),
                };
                write!(
                    f,
                    "stale-lock follows at {}: {} -> {} (flake.lock resolves to {}; run `nix flake lock`)",
                    location, source, declared_target, lock,
                )
            }
            ValidationError::FollowsDepthExceeded {
                edge,
                depth,
                max_depth,
                location,
            } => {
                write!(
                    f,
                    "follows depth exceeded at {}: {} -> {} reached depth {} (max {})",
                    location, edge.source, edge.follows, depth, max_depth
                )
            }
        }
    }
}

/// Result of validation containing any errors and warnings found.
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
                    source: AttrPath::parse("a.b").unwrap(),
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
