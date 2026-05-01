//! Follows-graph types: typed attribute paths and the edge graph built from them.
//!
//! An attribute path is a non-empty sequence of attribute names rooted at a
//! flake input. Segments are stored unquoted. The `"..."` quotes Nix requires
//! for names containing dots, leading digits, and similar live on the
//! rendering boundary.
pub mod graph;
pub mod path;

pub use graph::{
    Cycle, DEFAULT_MAX_DEPTH, Edge, EdgeOrigin, FollowsGraph, StaleLockDeclaration,
    TransitiveGroup, is_follows_reference_to_parent,
};
pub use path::{AttrPath, AttrPathParseError, Segment, SegmentError, strip_outer_quotes};
