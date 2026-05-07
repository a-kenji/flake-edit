//! Forge interactions: talking to GitHub / Gitea / Forgejo, choosing between
//! semver tags and channel branches, normalizing versions, and applying
//! pin/unpin updates to `flake.nix`.

pub mod api;
pub mod channel;
pub mod update;
pub mod version;
