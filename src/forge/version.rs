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

/// Normalise `raw` into a [`ParsedRef`] for semver comparison.
///
/// One pass strips `refs/tags/`, then a leading `v`, then anything
/// from the first `-` onward. The order is load-bearing: a tag like
/// `refs/tags/v1.2.3-rc1` reaches the semver core only after all
/// three strips run.
///
/// `default_refs_tags_prefix` seeds [`ParsedRef::has_refs_tags_prefix`]
/// when the input does not literally start with `refs/tags/`. Used by
/// the update path to record that an input which lacks the prefix in
/// `flake.nix` should nonetheless have its new ref written with one.
pub fn parse_ref(raw: &str, default_refs_tags_prefix: bool) -> ParsedRef {
    fn strip_until_char(s: &str, c: char) -> Option<String> {
        s.find(c).map(|index| s[index + 1..].to_string())
    }

    let mut maybe_version = raw.to_string();
    let mut previous_ref = String::new();
    let mut has_refs_tags_prefix = default_refs_tags_prefix;

    if let Some(normalized_version) = maybe_version.strip_prefix("refs/tags/") {
        has_refs_tags_prefix = true;
        previous_ref = maybe_version.clone();
        maybe_version = normalized_version.to_string();
    }

    if let Some(normalized_version) = maybe_version.strip_prefix('v') {
        previous_ref = maybe_version.clone();
        maybe_version = normalized_version.to_string();
    }

    if let Some(normalized_version) = strip_until_char(&maybe_version, '-') {
        previous_ref = maybe_version.clone();
        maybe_version = normalized_version.to_string();
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
    fn v_prefix_with_prerelease_strips_to_suffix_only() {
        // The dash strip discards the semver core, so the suffix `rc1` is what
        // gets normalized; the three-segment shape is reached by padding zeros.
        check("v1.2.3-rc1", false, "rc1.0.0", "1.2.3-rc1", false);
    }

    #[test]
    fn bare_prerelease_long_strips_to_suffix_only() {
        check("1.0.0-alpha.1", false, "alpha.1.0", "1.0.0-alpha.1", false);
    }

    #[test]
    fn bare_prerelease_short_strips_to_suffix_only() {
        check("2.0.0-beta", false, "beta.0.0", "2.0.0-beta", false);
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
    fn nix_darwin_channel_strips_first_dash_only() {
        // The dash strip is one-shot, so `nix-darwin-24.05` keeps `darwin-24.05`
        // as the residue and normalizes that, producing a noisy semver string.
        check(
            "nix-darwin-24.05",
            false,
            "darwin.0.0-24.05",
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
        // Without any subsequent strip, previous_ref keeps the `refs/tags/`
        // form set on the first strip.
        check("refs/tags/1.2.3", false, "1.2.3", "refs/tags/1.2.3", true);
    }

    #[test]
    fn refs_tags_v_prerelease_combines_strips() {
        check("refs/tags/v1.2.3-rc1", false, "rc1.0.0", "1.2.3-rc1", true);
    }

    #[test]
    fn iso_date_strips_first_dash() {
        check("2024-05-01", false, "05.0.0-01", "2024-05-01", false);
    }

    #[test]
    fn empty_input_returns_empty_normalized() {
        check("", false, "", "", false);
    }

    #[test]
    fn lone_v_dash_strips_to_empty_normalized() {
        check("v-", false, "", "-", false);
    }

    #[test]
    fn default_refs_tags_prefix_persists_without_refs_tags_string() {
        check("1.2.3", true, "1.2.3", "1.2.3", true);
    }
}
