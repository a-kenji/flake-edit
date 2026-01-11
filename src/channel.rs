//! Channel-based update logic for repos like nixpkgs, home-manager, and nix-darwin.
//!
//! These repos use branches (e.g., `nixos-24.11`, `nixpkgs-unstable`) instead of
//! semver tags for versioning.

use crate::api::{Branches, branch_exists, get_branches};

/// Update strategy for a given input.
#[derive(Debug, Clone, PartialEq)]
pub enum UpdateStrategy {
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
pub enum ChannelType {
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
    pub fn is_unstable(&self) -> bool {
        matches!(self, ChannelType::Unstable)
    }

    /// Returns the version tuple for comparison, if applicable.
    pub fn version(&self) -> Option<(u32, u32)> {
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
    pub fn prefix(&self) -> Option<&'static str> {
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
pub fn detect_strategy(owner: &str, repo: &str) -> UpdateStrategy {
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
pub fn parse_channel_ref(ref_str: &str) -> ChannelType {
    let ref_str = ref_str.strip_prefix("refs/heads/").unwrap_or(ref_str);

    if ref_str == "nixos-unstable" || ref_str == "nixpkgs-unstable" {
        return ChannelType::Unstable;
    }

    // master/main are unstable branches for home-manager and nix-darwin
    if ref_str == "master" || ref_str == "main" {
        return ChannelType::Unstable;
    }

    if let Some(version) = ref_str.strip_prefix("nixos-") {
        if let Some((year, month)) = parse_version(version) {
            return ChannelType::NixosStable { year, month };
        }
    }

    if let Some(version) = ref_str.strip_prefix("nixpkgs-") {
        if let Some((year, month)) = parse_version(version) {
            return ChannelType::NixpkgsStable { year, month };
        }
    }

    if let Some(version) = ref_str.strip_prefix("release-") {
        if let Some((year, month)) = parse_version(version) {
            return ChannelType::HomeManagerRelease { year, month };
        }
    }

    if let Some(version) = ref_str.strip_prefix("nix-darwin-") {
        if let Some((year, month)) = parse_version(version) {
            return ChannelType::NixDarwinStable { year, month };
        }
    }

    // Try bare version YY.MM
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
/// Returns `None` if:
/// - The current ref is unstable (should not be updated)
/// - The current ref is not a recognized channel
/// - No newer channel exists
pub fn find_latest_channel(
    current_ref: &str,
    owner: &str,
    repo: &str,
    domain: Option<&str>,
) -> Option<String> {
    let current_channel = parse_channel_ref(current_ref);

    // Don't update unstable channels
    if current_channel.is_unstable() {
        tracing::debug!("Skipping update for unstable channel: {}", current_ref);
        return None;
    }

    // Only update recognized channel patterns
    let prefix = current_channel.prefix()?;
    let current_version = current_channel.version()?;

    // Try targeted approach first (much faster for repos with many branches like nixpkgs)
    if let Some(latest) = find_latest_channel_targeted(prefix, current_version, owner, repo, domain)
    {
        if latest != current_ref {
            return Some(latest);
        } else {
            tracing::debug!("{} is already on the latest channel", current_ref);
            return None;
        }
    }

    // Fallback: fetch all branches (for Gitea/Forgejo or if targeted fails)
    tracing::debug!("Targeted lookup failed, falling back to listing all branches");
    let branches = match get_branches(repo, owner, domain) {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("Failed to fetch branches: {}", e);
            return None;
        }
    };

    // Find all channels matching the same prefix and pick the latest
    let latest = find_latest_matching_branch(&branches, prefix, current_version);

    if let Some(ref latest_branch) = latest {
        if latest_branch == current_ref {
            tracing::debug!("{} is already on the latest channel", current_ref);
            return None;
        }
    }

    latest
}

/// Generate candidate channel versions from current to ~5 years in the future.
/// Returns candidates from NEWEST to OLDEST for early exit optimization.
fn generate_candidate_channels(prefix: &str, current_version: (u32, u32)) -> Vec<String> {
    let (current_year, current_month) = current_version;
    let mut candidates = Vec::new();

    // Generate candidates for the next ~5 years (10 releases)
    // NixOS releases in May (05) and November (11)
    let mut year = current_year;
    let mut month = current_month;

    for _ in 0..10 {
        // Move to next release
        if month == 5 {
            month = 11;
        } else {
            month = 5;
            year += 1;
        }

        candidates.push(format!("{}{}.{:02}", prefix, year, month));
    }

    // Reverse so we check newest first (for early exit)
    candidates.reverse();
    candidates
}

/// Try to find the latest channel using targeted branch existence checks.
/// Returns None if no candidates exist (caller should fall back to listing).
fn find_latest_channel_targeted(
    prefix: &str,
    current_version: (u32, u32),
    owner: &str,
    repo: &str,
    domain: Option<&str>,
) -> Option<String> {
    let candidates = generate_candidate_channels(prefix, current_version);

    tracing::debug!(
        "Checking candidate channels (newest first): {:?}",
        candidates
    );

    // Check from newest to oldest, return first match (will be the newest)
    for candidate in &candidates {
        tracing::debug!("Checking if branch exists: {}", candidate);
        if branch_exists(repo, owner, candidate, domain) {
            tracing::debug!("Found existing channel: {}", candidate);
            return Some(candidate.clone());
        }
    }

    // No newer channel found, check if current version exists
    let current_branch = format!("{}{}.{:02}", prefix, current_version.0, current_version.1);
    tracing::debug!("No newer channel, checking current: {}", current_branch);
    if branch_exists(repo, owner, &current_branch, domain) {
        return Some(current_branch);
    }

    None
}

/// Find the latest branch matching a given prefix that is newer than current_version.
fn find_latest_matching_branch(
    branches: &Branches,
    prefix: &str,
    current_version: (u32, u32),
) -> Option<String> {
    let mut best: Option<(u32, u32, String)> = None;

    for branch_name in &branches.names {
        // Check if this branch matches our prefix
        if let Some(version_str) = branch_name.strip_prefix(prefix) {
            // Skip unstable variants
            if version_str == "unstable" {
                continue;
            }

            if let Some((year, month)) = parse_version(version_str) {
                // Only consider versions >= current
                if (year, month) >= current_version {
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
