//! Channel-based update logic for repos like nixpkgs, home-manager, and nix-darwin.
//!
//! These repos use branches (e.g., `nixos-24.11`, `nixpkgs-unstable`) instead of
//! semver tags for versioning.

use super::api::{ApiError, Branches, ForgeClient};

/// Update strategy for a given input.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum UpdateStrategy {
    /// Standard semver tag-based updates (most repos)
    SemverTags,
    /// nixpkgs channel-based updates (nixos-YY.MM, nixpkgs-YY.MM)
    NixpkgsChannel,
    /// home-manager channel-based updates (release-YY.MM)
    HomeManagerChannel,
    /// nix-darwin channel-based updates (nix-darwin-YY.MM)
    NixDarwinChannel,
}

/// Detected channel type from a ref string.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ChannelType {
    /// nixos-YY.MM (e.g., nixos-24.11)
    NixosStable { year: u32, month: u32 },
    /// nixpkgs-YY.MM (e.g., nixpkgs-24.11)
    NixpkgsStable { year: u32, month: u32 },
    /// Unstable/rolling branches (nixos-unstable, nixpkgs-unstable, master, main)
    Unstable,
    /// home-manager release-YY.MM
    HomeManagerRelease { year: u32, month: u32 },
    /// nix-darwin-YY.MM
    NixDarwinStable { year: u32, month: u32 },
    /// Bare version YY.MM (no prefix)
    BareVersion { year: u32, month: u32 },
    /// Not a recognized channel pattern
    Unknown,
}

impl ChannelType {
    /// Returns true if this is an unstable/rolling channel that shouldn't be updated.
    pub(crate) fn is_unstable(&self) -> bool {
        matches!(self, ChannelType::Unstable)
    }

    /// Returns the version tuple for comparison, if applicable.
    pub(crate) fn version(&self) -> Option<(u32, u32)> {
        match self {
            ChannelType::NixosStable { year, month }
            | ChannelType::NixpkgsStable { year, month }
            | ChannelType::HomeManagerRelease { year, month }
            | ChannelType::NixDarwinStable { year, month }
            | ChannelType::BareVersion { year, month } => Some((*year, *month)),
            _ => None,
        }
    }

    /// Returns the branch prefix for finding similar channels.
    pub(crate) fn prefix(&self) -> Option<&'static str> {
        match self {
            ChannelType::NixosStable { .. } => Some("nixos-"),
            ChannelType::NixpkgsStable { .. } => Some("nixpkgs-"),
            ChannelType::HomeManagerRelease { .. } => Some("release-"),
            ChannelType::NixDarwinStable { .. } => Some("nix-darwin-"),
            ChannelType::BareVersion { .. } => Some(""),
            _ => None,
        }
    }
}

/// Detect the update strategy based on owner/repo.
pub(crate) fn detect_strategy(owner: &str, repo: &str) -> UpdateStrategy {
    match (owner.to_lowercase().as_str(), repo.to_lowercase().as_str()) {
        ("nixos", "nixpkgs") => UpdateStrategy::NixpkgsChannel,
        ("nix-community", "home-manager") => UpdateStrategy::HomeManagerChannel,
        ("lnl7", "nix-darwin") | ("nix-community", "nix-darwin") => {
            UpdateStrategy::NixDarwinChannel
        }
        _ => UpdateStrategy::SemverTags,
    }
}

/// Parse a ref string to determine its channel type.
pub(crate) fn parse_channel_ref(ref_str: &str) -> ChannelType {
    let ref_str = ref_str.strip_prefix("refs/heads/").unwrap_or(ref_str);

    if ref_str == "nixos-unstable" || ref_str == "nixpkgs-unstable" {
        return ChannelType::Unstable;
    }

    // master/main are unstable branches for home-manager and nix-darwin
    if ref_str == "master" || ref_str == "main" {
        return ChannelType::Unstable;
    }

    if let Some(version) = ref_str.strip_prefix("nixos-")
        && let Some((year, month)) = parse_version(version)
    {
        return ChannelType::NixosStable { year, month };
    }

    if let Some(version) = ref_str.strip_prefix("nixpkgs-")
        && let Some((year, month)) = parse_version(version)
    {
        return ChannelType::NixpkgsStable { year, month };
    }

    if let Some(version) = ref_str.strip_prefix("release-")
        && let Some((year, month)) = parse_version(version)
    {
        return ChannelType::HomeManagerRelease { year, month };
    }

    if let Some(version) = ref_str.strip_prefix("nix-darwin-")
        && let Some((year, month)) = parse_version(version)
    {
        return ChannelType::NixDarwinStable { year, month };
    }

    if let Some((year, month)) = parse_version(ref_str) {
        return ChannelType::BareVersion { year, month };
    }

    ChannelType::Unknown
}

/// Parse a version string like "24.11" into (year, month).
fn parse_version(version: &str) -> Option<(u32, u32)> {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() == 2 {
        let year = parts[0].parse::<u32>().ok()?;
        let month = parts[1].parse::<u32>().ok()?;
        // Sanity check: year should be reasonable (20-99), month should be valid
        if (20..=99).contains(&year) && (month == 5 || month == 11) {
            return Some((year, month));
        }
    }
    None
}

/// Find the latest channel branch that matches the current channel type.
///
/// Returns `Ok(None)` if:
/// - The current ref is unstable (should not be updated)
/// - The current ref is not a recognized channel
/// - No newer channel exists and we are already on the latest
///
/// Returns `Err` when a transient forge failure (timeout, DNS,
/// 5xx, ...) prevents us from proving anything about the candidate
/// set. Without this, a flaky network looked exactly like "no
/// newer channel exists" and silently kept the user pinned to a
/// stale release.
pub(crate) fn find_latest_channel(
    client: &ForgeClient,
    current_ref: &str,
    owner: &str,
    repo: &str,
    domain: Option<&str>,
) -> Result<Option<String>, ApiError> {
    let current_channel = parse_channel_ref(current_ref);

    if current_channel.is_unstable() {
        tracing::debug!("Skipping update for unstable channel: {}", current_ref);
        return Ok(None);
    }

    let (prefix, current_version) = match (current_channel.prefix(), current_channel.version()) {
        (Some(p), Some(v)) => (p, v),
        _ => return Ok(None),
    };

    // Targeted candidate probing is cheap on repos with many
    // branches (nixpkgs has thousands); the all-branches list is the
    // fallback for forges or transient conditions where targeted
    // returned `Ok(None)`.
    if let Some(latest) =
        find_latest_channel_targeted(client, prefix, current_version, owner, repo, domain)?
    {
        if latest != current_ref {
            return Ok(Some(latest));
        } else {
            tracing::debug!("{} is already on the latest channel", current_ref);
            return Ok(None);
        }
    }

    tracing::debug!("Targeted lookup failed, falling back to listing all branches");
    let branches = client.list_branches(owner, repo, domain)?;
    let latest = find_latest_matching_branch(&branches, prefix, current_version);

    if let Some(ref latest_branch) = latest
        && latest_branch == current_ref
    {
        tracing::debug!("{} is already on the latest channel", current_ref);
        return Ok(None);
    }

    Ok(latest)
}

/// Every branch name [`find_latest_channel_targeted`] may probe for
/// a given `(prefix, current_version)` start: the future candidates
/// plus the current branch itself. Used by the GraphQL batch warmer
/// in `api.rs` to pre-populate `branch_exists_cache` so the targeted
/// loop later short-circuits to cache hits.
pub(crate) fn channel_probe_candidates(prefix: &str, current_version: (u32, u32)) -> Vec<String> {
    let mut all = generate_candidate_channels(prefix, current_version);
    all.push(format!(
        "{}{}.{:02}",
        prefix, current_version.0, current_version.1
    ));
    all
}

/// Generate candidate channel versions from current to ~5 years in the future.
/// Returns candidates from NEWEST to OLDEST for early exit optimization.
fn generate_candidate_channels(prefix: &str, current_version: (u32, u32)) -> Vec<String> {
    let (current_year, current_month) = current_version;
    let mut candidates = Vec::new();

    // NixOS cuts a release in May (.05) and November (.11). Ten
    // iterations covers about five years of future cuts; far enough
    // ahead that we never need a refresh, close enough that the
    // candidate set stays small.
    let mut year = current_year;
    let mut month = current_month;

    for _ in 0..10 {
        if month == 5 {
            month = 11;
        } else {
            month = 5;
            year += 1;
        }

        candidates.push(format!("{}{}.{:02}", prefix, year, month));
    }

    // Newest first so [`find_latest_channel_targeted`] can early-exit
    // on the first hit and avoid probing older candidates.
    candidates.reverse();
    candidates
}

/// Find the latest channel by probing candidate branches one by
/// one. Cheaper than listing all branches; falls back to that on
/// `Ok(None)`. `Err` always propagates. See [`find_latest_channel`]
/// for the full contract.
fn find_latest_channel_targeted(
    client: &ForgeClient,
    prefix: &str,
    current_version: (u32, u32),
    owner: &str,
    repo: &str,
    domain: Option<&str>,
) -> Result<Option<String>, ApiError> {
    let candidates = generate_candidate_channels(prefix, current_version);

    tracing::debug!(
        "Checking candidate channels (newest first): {:?}",
        candidates
    );

    for candidate in &candidates {
        tracing::debug!("Checking if branch exists: {}", candidate);
        if client.branch_exists(owner, repo, candidate, domain)? {
            tracing::debug!("Found existing channel: {}", candidate);
            return Ok(Some(candidate.clone()));
        }
    }

    let current_branch = format!("{}{}.{:02}", prefix, current_version.0, current_version.1);
    tracing::debug!("No newer channel, checking current: {}", current_branch);
    if client.branch_exists(owner, repo, &current_branch, domain)? {
        return Ok(Some(current_branch));
    }

    Ok(None)
}

/// Find the latest branch matching a given prefix that is newer than current_version.
fn find_latest_matching_branch(
    branches: &Branches,
    prefix: &str,
    current_version: (u32, u32),
) -> Option<String> {
    let mut best: Option<(u32, u32, String)> = None;

    for branch_name in &branches.names {
        if let Some(version_str) = branch_name.strip_prefix(prefix) {
            // `nixos-unstable` / `nixpkgs-unstable` share the prefix but
            // are rolling branches; updating to one of them would point
            // a release pin at a moving target.
            if version_str == "unstable" {
                continue;
            }

            if let Some((year, month)) = parse_version(version_str)
                && (year, month) >= current_version
            {
                match &best {
                    None => {
                        best = Some((year, month, branch_name.clone()));
                    }
                    Some((best_year, best_month, _)) => {
                        if (year, month) > (*best_year, *best_month) {
                            best = Some((year, month, branch_name.clone()));
                        }
                    }
                }
            }
        }
    }

    best.map(|(_, _, name)| name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_strategy() {
        assert_eq!(
            detect_strategy("nixos", "nixpkgs"),
            UpdateStrategy::NixpkgsChannel
        );
        assert_eq!(
            detect_strategy("NixOS", "nixpkgs"),
            UpdateStrategy::NixpkgsChannel
        );
        assert_eq!(
            detect_strategy("nix-community", "home-manager"),
            UpdateStrategy::HomeManagerChannel
        );
        assert_eq!(
            detect_strategy("LnL7", "nix-darwin"),
            UpdateStrategy::NixDarwinChannel
        );
        assert_eq!(
            detect_strategy("nix-community", "nix-darwin"),
            UpdateStrategy::NixDarwinChannel
        );
        assert_eq!(
            detect_strategy("some-user", "some-repo"),
            UpdateStrategy::SemverTags
        );
    }

    #[test]
    fn test_parse_channel_ref_nixos() {
        assert_eq!(
            parse_channel_ref("nixos-24.11"),
            ChannelType::NixosStable {
                year: 24,
                month: 11
            }
        );
        assert_eq!(
            parse_channel_ref("nixos-25.05"),
            ChannelType::NixosStable { year: 25, month: 5 }
        );
        assert_eq!(parse_channel_ref("nixos-unstable"), ChannelType::Unstable);
    }

    #[test]
    fn test_parse_channel_ref_nixpkgs() {
        assert_eq!(
            parse_channel_ref("nixpkgs-24.11"),
            ChannelType::NixpkgsStable {
                year: 24,
                month: 11
            }
        );
        assert_eq!(parse_channel_ref("nixpkgs-unstable"), ChannelType::Unstable);
    }

    #[test]
    fn test_parse_channel_ref_home_manager() {
        assert_eq!(
            parse_channel_ref("release-24.11"),
            ChannelType::HomeManagerRelease {
                year: 24,
                month: 11
            }
        );
        assert_eq!(parse_channel_ref("master"), ChannelType::Unstable);
    }

    #[test]
    fn test_parse_channel_ref_nix_darwin() {
        assert_eq!(
            parse_channel_ref("nix-darwin-24.11"),
            ChannelType::NixDarwinStable {
                year: 24,
                month: 11
            }
        );
        assert_eq!(parse_channel_ref("main"), ChannelType::Unstable);
    }

    #[test]
    fn test_parse_channel_ref_bare_version() {
        assert_eq!(
            parse_channel_ref("24.11"),
            ChannelType::BareVersion {
                year: 24,
                month: 11
            }
        );
        assert_eq!(
            parse_channel_ref("25.05"),
            ChannelType::BareVersion { year: 25, month: 5 }
        );
    }

    #[test]
    fn test_parse_channel_ref_with_refs_heads_prefix() {
        assert_eq!(
            parse_channel_ref("refs/heads/nixos-24.11"),
            ChannelType::NixosStable {
                year: 24,
                month: 11
            }
        );
    }

    #[test]
    fn test_parse_channel_ref_unknown() {
        assert_eq!(parse_channel_ref("v1.0.0"), ChannelType::Unknown);
        assert_eq!(parse_channel_ref("nixos-invalid"), ChannelType::Unknown);
        assert_eq!(parse_channel_ref("feature-branch"), ChannelType::Unknown);
    }

    #[test]
    fn test_channel_type_is_unstable() {
        assert!(ChannelType::Unstable.is_unstable());
        assert!(parse_channel_ref("master").is_unstable());
        assert!(parse_channel_ref("main").is_unstable());
        assert!(parse_channel_ref("nixos-unstable").is_unstable());
        assert!(
            !ChannelType::NixosStable {
                year: 24,
                month: 11
            }
            .is_unstable()
        );
    }

    #[test]
    fn test_find_latest_matching_branch() {
        let branches = Branches {
            names: vec![
                "nixos-23.11".to_string(),
                "nixos-24.05".to_string(),
                "nixos-24.11".to_string(),
                "nixos-unstable".to_string(),
                "master".to_string(),
            ],
        };

        // Should find 24.11 as latest when on 24.05
        let result = find_latest_matching_branch(&branches, "nixos-", (24, 5));
        assert_eq!(result, Some("nixos-24.11".to_string()));

        // Should return current when already on latest
        let result = find_latest_matching_branch(&branches, "nixos-", (24, 11));
        assert_eq!(result, Some("nixos-24.11".to_string()));

        // Should return None when on a version newer than anything available
        let result = find_latest_matching_branch(&branches, "nixos-", (25, 5));
        assert_eq!(result, None);
    }

    #[test]
    fn test_generate_candidate_channels() {
        // Starting from 24.05: 24.11, 25.05, 25.11, ..., 29.05 (10 items)
        // Reversed: 29.05, 28.11, ..., 24.11
        let candidates = generate_candidate_channels("nixos-", (24, 5));
        assert_eq!(candidates.len(), 10);
        // Newest first
        assert_eq!(candidates[0], "nixos-29.05");
        // Oldest last
        assert_eq!(candidates[9], "nixos-24.11");

        // Starting from 24.11: 25.05, 25.11, ..., 29.11 (10 items)
        // Reversed: 29.11, 29.05, ..., 25.05
        let candidates = generate_candidate_channels("nixpkgs-", (24, 11));
        assert_eq!(candidates[0], "nixpkgs-29.11"); // newest
        assert_eq!(candidates[9], "nixpkgs-25.05"); // oldest
    }
}
