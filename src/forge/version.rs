//! Ref-string normalisation shared by the forge tag-listing path
//! and the update-strategy classifier.
//!
//! Tag schemes in the wild are inconsistent (`v1.2.3`,
//! `refs/tags/v1.2.3`, `release-24.05`, bare `1.0`). [`parse_ref`]
//! reduces each input to a [`semver::Version`]-parseable string plus
//! the metadata needed to put any stripped prefix back when the
//! update writes the new ref into `flake.nix`.

/// Outcome of normalising a ref string for semver comparison.
#[derive(Debug, Clone)]
pub struct ParsedRef {
    /// The input passed to [`parse_ref`], verbatim.
    pub original_ref: String,
    /// Ref reduced to a shape [`semver::Version::parse`] accepts
    /// (or an unparseable residue when the input was not a semver
    /// tag).
    pub normalized_for_semver: String,
    /// Form of the input after one round of stripping. Surfaces in
    /// `Updater`'s "from X to Y" status output, where the user
    /// expects the displayed previous ref to match what they
    /// originally pinned rather than the fully-normalised core.
    pub previous_ref: String,
    /// `true` when the input carried a `refs/tags/` prefix (or the
    /// caller forced it via `default_refs_tags_prefix`). Callers
    /// reattach the prefix to the newly-resolved tag to preserve
    /// the user's existing ref style.
    pub has_refs_tags_prefix: bool,
}

pub(crate) fn normalize_semver(tag: &str) -> String {
    let (core, suffix) = tag
        .find(|c| ['-', '+'].contains(&c))
        .map(|idx| (&tag[..idx], &tag[idx..]))
        .unwrap_or((tag, ""));
    if core.is_empty() {
        return tag.to_string();
    }
    let dot_count = core.matches('.').count();
    let normalized_core = match dot_count {
        0 => format!("{core}.0.0"),
        1 => format!("{core}.0"),
        _ => core.to_string(),
    };
    format!("{normalized_core}{suffix}")
}

/// Returns `true` when `proposed` parses as a strictly lower
/// version than `current` under semver precedence.
pub fn is_downgrade(current: &str, proposed: &str) -> bool {
    let cur = parse_ref(current, false);
    let prop = parse_ref(proposed, false);
    match (
        semver::Version::parse(&cur.normalized_for_semver),
        semver::Version::parse(&prop.normalized_for_semver),
    ) {
        (Ok(c), Ok(p)) => p.cmp_precedence(&c) == std::cmp::Ordering::Less,
        _ => false,
    }
}

/// Normalise `raw` into a [`ParsedRef`] for semver comparison.
///
/// Strips `refs/tags/` and then any non-digit scheme prefix that
/// precedes the first digit, so `v`, `hl`, `release-`, and
/// `nix-darwin-` are all reduced to their numeric core in one pass.
/// The remainder is fed through [`normalize_semver`], which pads
/// short forms like `1.0` out to three segments.
pub fn parse_ref(raw: &str, default_refs_tags_prefix: bool) -> ParsedRef {
    let mut maybe_version = raw.to_string();
    let mut previous_ref = String::new();
    let mut has_refs_tags_prefix = default_refs_tags_prefix;

    if let Some(stripped) = maybe_version.strip_prefix("refs/tags/") {
        has_refs_tags_prefix = true;
        previous_ref = maybe_version.clone();
        maybe_version = stripped.to_string();
    }

    if let Some(digit_idx) = maybe_version.find(|c: char| c.is_ascii_digit())
        && digit_idx > 0
    {
        previous_ref = maybe_version.clone();
        maybe_version = maybe_version[digit_idx..].to_string();
    }

    if previous_ref.is_empty() {
        previous_ref = maybe_version.clone();
    }

    ParsedRef {
        original_ref: raw.to_string(),
        normalized_for_semver: normalize_semver(&maybe_version),
        previous_ref,
        has_refs_tags_prefix,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(
        raw: &str,
        default_refs_tags_prefix: bool,
        expected_normalized: &str,
        expected_previous: &str,
        expected_has_prefix: bool,
    ) {
        let parsed = parse_ref(raw, default_refs_tags_prefix);
        assert_eq!(parsed.original_ref, raw, "original_ref for {raw:?}");
        assert_eq!(
            parsed.normalized_for_semver, expected_normalized,
            "normalized_for_semver for {raw:?}"
        );
        assert_eq!(
            parsed.previous_ref, expected_previous,
            "previous_ref for {raw:?}"
        );
        assert_eq!(
            parsed.has_refs_tags_prefix, expected_has_prefix,
            "has_refs_tags_prefix for {raw:?}"
        );
    }

    #[test]
    fn bare_three_segment_passes_through() {
        check("1.2.3", false, "1.2.3", "1.2.3", false);
    }

    #[test]
    fn bare_two_segment_pads_patch() {
        check("1.0", false, "1.0.0", "1.0", false);
    }

    #[test]
    fn v_prefix_normalizes_to_semver() {
        check("v1.2.3", false, "1.2.3", "v1.2.3", false);
    }

    #[test]
    fn v_prefix_with_prerelease_keeps_semver_core() {
        check("v1.2.3-rc1", false, "1.2.3-rc1", "v1.2.3-rc1", false);
    }

    #[test]
    fn bare_prerelease_long_passes_through_as_semver() {
        check(
            "1.0.0-alpha.1",
            false,
            "1.0.0-alpha.1",
            "1.0.0-alpha.1",
            false,
        );
    }

    #[test]
    fn bare_prerelease_short_passes_through_as_semver() {
        check("2.0.0-beta", false, "2.0.0-beta", "2.0.0-beta", false);
    }

    #[test]
    fn hl_prefixed_tag_keeps_version_core_as_prerelease() {
        check("hl0.47.0-1", false, "0.47.0-1", "hl0.47.0-1", false);
    }

    #[test]
    fn plus_metadata_passes_through() {
        check("1.2.3+gitea", false, "1.2.3+gitea", "1.2.3+gitea", false);
    }

    #[test]
    fn plus_metadata_dotted_passes_through() {
        check("1.2.3+meta.1", false, "1.2.3+meta.1", "1.2.3+meta.1", false);
    }

    #[test]
    fn release_channel_strips_first_dash() {
        check("release-24.05", false, "24.05.0", "release-24.05", false);
    }

    #[test]
    fn nix_darwin_channel_strips_full_prefix() {
        // `05` has a leading zero, so `Version::parse` rejects this
        // core. That rejection is what stops the `forge::api`
        // cheap-path predicate from treating channel branches as
        // semver tags.
        check(
            "nix-darwin-24.05",
            false,
            "24.05.0",
            "nix-darwin-24.05",
            false,
        );
    }

    #[test]
    fn refs_tags_v_prefix_records_prefix_and_strips_v() {
        check("refs/tags/v1.0.0", false, "1.0.0", "v1.0.0", true);
    }

    #[test]
    fn refs_tags_bare_keeps_full_previous_ref() {
        check("refs/tags/1.2.3", false, "1.2.3", "refs/tags/1.2.3", true);
    }

    #[test]
    fn refs_tags_v_prerelease_keeps_semver_core() {
        check(
            "refs/tags/v1.2.3-rc1",
            false,
            "1.2.3-rc1",
            "v1.2.3-rc1",
            true,
        );
    }

    #[test]
    fn iso_date_pads_year_into_semver_with_date_prerelease() {
        // Inputs that start with a digit skip the prefix strip and
        // the year becomes the major, so semver ordering still
        // picks the most recent date out of a date-shaped tag list.
        check("2024-05-01", false, "2024.0.0-05-01", "2024-05-01", false);
    }

    #[test]
    fn empty_input_returns_empty_normalized() {
        check("", false, "", "", false);
    }

    #[test]
    fn lone_v_dash_has_no_digit_to_anchor_strip() {
        // Pathological inputs are intentionally normalised into a
        // shape `Version::parse` rejects, so they get dropped
        // downstream rather than masquerading as a valid version.
        check("v-", false, "v.0.0-", "v-", false);
    }

    #[test]
    fn default_refs_tags_prefix_persists_without_refs_tags_string() {
        check("1.2.3", true, "1.2.3", "1.2.3", true);
    }

    #[test]
    fn is_downgrade_flags_lower_hl_prefixed_proposal() {
        assert!(is_downgrade("hl0.47.0-1", "hl0.33.0-1"));
    }

    #[test]
    fn is_downgrade_allows_strictly_greater_proposal() {
        assert!(!is_downgrade("hl0.33.0-1", "hl0.47.0-1"));
        assert!(!is_downgrade("v1.0.0", "v2.0.0"));
    }

    #[test]
    fn is_downgrade_allows_equal_versions() {
        // Equal versions are not a downgrade. The existing "already
        // on the latest" path handles that case; the guard must not
        // pre-empt it.
        assert!(!is_downgrade("v1.2.3", "v1.2.3"));
        assert!(!is_downgrade("1.0.0", "v1.0.0"));
    }

    #[test]
    fn is_downgrade_returns_false_when_either_side_unparseable() {
        // Non-semver pins (commit hash, branch name) leave the
        // ordering question undefined; the guard must defer to the
        // existing flow rather than silently dropping the update.
        assert!(!is_downgrade("not-a-version", "1.2.3"));
        assert!(!is_downgrade("1.2.3", "not-a-version"));
        assert!(!is_downgrade("", "1.2.3"));
    }
}
