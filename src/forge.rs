//! Forge interactions: talking to GitHub / Gitea / Forgejo, choosing between
//! semver tags and channel branches, normalizing versions, and applying
//! pin/unpin updates to `flake.nix`.

pub mod api;
pub(crate) mod archive;
pub(crate) mod cache;
pub mod channel;
pub(crate) mod channel_tarball;
#[cfg(test)]
pub(crate) mod testing;
pub mod update;
pub mod version;
