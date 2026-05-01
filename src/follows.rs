//! Typed representation of follows-graph attribute paths.
//!
//! The grammar of a follows-graph attribute path is a non-empty sequence of
//! attribute names rooted at a flake input. This module owns the canonical
//! types ([`Segment`], [`AttrPath`]) and the in-memory invariant that every
//! segment is stored unquoted: the surface-level `"..."` quotes that Nix
//! requires for names containing dots, hyphens after digits, etc., live on
//! the rendering boundary, not the value.
pub mod graph;
pub mod path;

pub use graph::{
    Cycle, DEFAULT_MAX_DEPTH, Edge, EdgeOrigin, FollowsGraph, GraphError, StaleLockDeclaration,
    TransitiveGroup, is_follows_reference_to_parent,
};
pub use path::{AttrPath, AttrPathParseError, Segment, SegmentError};
