//! Parsing and resolution for nixos.org channel tarball inputs, e.g.
//! `https://channels.nixos.org/nixos-26.05/nixexprs.tar.xz`.

use super::api::ApiError;
use super::channel::generate_candidate_channels;

/// Exact `(host, path prefix)` allowlist. Anything else is left alone.
const CHANNEL_ENDPOINTS: [(&str, &str); 2] =
    [("channels.nixos.org", ""), ("nixos.org", "channels")];

/// `nixexprs.tar.bz2` is excluded: the channel server 404s on it.
const CHANNEL_FILE: &str = "nixexprs.tar.xz";

/// `release-`/`nix-darwin-` are git branches, not published channels.
const CHANNEL_PREFIXES: [&str; 2] = ["nixos-", "nixpkgs-"];

const CHANNEL_VARIANT_SUFFIXES: [&str; 3] = ["", "-small", "-darwin"];

/// A parsed stable channel token, e.g. `nixos-26.05-small`. Unstable
/// tokens do not parse and are never bumped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChannelToken {
    prefix: String,
    version: (u32, u32),
    suffix: String,
}

impl ChannelToken {
    pub(crate) fn parse(token: &str) -> Option<ChannelToken> {
        let prefix = CHANNEL_PREFIXES
            .into_iter()
            .find(|candidate| token.starts_with(candidate))?;
        let rest = &token[prefix.len()..];

        // The empty suffix matches every token, so longest match wins.
        let suffix = CHANNEL_VARIANT_SUFFIXES
            .into_iter()
            .filter(|candidate| rest.ends_with(candidate))
            .max_by_key(|candidate| candidate.len())?;
        let version = &rest[..rest.len() - suffix.len()];

        Some(ChannelToken {
            prefix: prefix.to_string(),
            version: parse_release(version)?,
            suffix: suffix.to_string(),
        })
    }

    /// Newest first, current release excluded.
    fn future_candidates(&self) -> Vec<String> {
        generate_candidate_channels(&self.prefix, self.version)
            .into_iter()
            .map(|candidate| format!("{candidate}{}", self.suffix))
            .collect()
    }
}

/// Parse a two-digit `YY.MM` release token into `(year, month)`.
/// NixOS releases only in May and November.
fn parse_release(version: &str) -> Option<(u32, u32)> {
    let (year, month) = version.split_once('.')?;
    if year.len() != 2 || month.len() != 2 {
        return None;
    }
    let year = year.parse::<u32>().ok()?;
    let month = month.parse::<u32>().ok()?;
    if !(20..=99).contains(&year) || !(month == 5 || month == 11) {
        return None;
    }
    Some((year, month))
}

/// A parsed channel tarball URL, stored verbatim so
/// [`ChannelTarballUrl::with_channel`] only swaps the channel token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChannelTarballUrl {
    scheme: String,
    host: String,
    /// `""` for `channels.nixos.org`, `channels` for `nixos.org`.
    path_prefix: String,
    channel: String,
    /// Trailing `?query`, including the leading `?`.
    tail: String,
}

impl ChannelTarballUrl {
    pub(crate) fn parse(uri: &str) -> Option<ChannelTarballUrl> {
        let (scheme, rest) = uri.split_once("://")?;
        if scheme != "http" && scheme != "https" {
            return None;
        }

        let (host, path_and_tail) = rest.split_once('/')?;

        // Split the query off first: a `/` inside it is not a separator.
        let (path, tail) = match path_and_tail.find('?') {
            Some(idx) => (&path_and_tail[..idx], &path_and_tail[idx..]),
            None => (path_and_tail, ""),
        };

        let path_prefix = CHANNEL_ENDPOINTS
            .into_iter()
            .find_map(|(known_host, prefix)| (known_host == host).then_some(prefix))?;

        let segments: Vec<&str> = path.split('/').collect();
        let expected = if path_prefix.is_empty() { 2 } else { 3 };
        if segments.len() != expected {
            return None;
        }
        if !path_prefix.is_empty() && segments[0] != path_prefix {
            return None;
        }

        let channel = segments[expected - 2];
        let file = segments[expected - 1];
        if channel.is_empty() || file != CHANNEL_FILE {
            return None;
        }

        Some(ChannelTarballUrl {
            scheme: scheme.to_string(),
            host: host.to_string(),
            path_prefix: path_prefix.to_string(),
            channel: channel.to_string(),
            tail: tail.to_string(),
        })
    }

    pub(crate) fn channel(&self) -> &str {
        &self.channel
    }

    fn base(&self) -> String {
        if self.path_prefix.is_empty() {
            format!("{}://{}", self.scheme, self.host)
        } else {
            format!("{}://{}/{}", self.scheme, self.host, self.path_prefix)
        }
    }

    /// Targets the tarball, not the channel directory: the directory
    /// answers 200 even where the tarball under it 404s.
    pub(crate) fn probe_url(&self, channel: &str) -> String {
        format!("{}/{}/{}", self.base(), channel, CHANNEL_FILE)
    }

    pub(crate) fn with_channel(&self, channel: &str) -> String {
        format!("{}{}", self.probe_url(channel), self.tail)
    }
}

/// Resolve the newest published channel strictly newer than `url`'s.
pub(crate) fn find_latest_published_channel(
    url: &ChannelTarballUrl,
    mut exists: impl FnMut(&str) -> Result<bool, ApiError>,
) -> Result<Option<String>, ApiError> {
    let Some(token) = ChannelToken::parse(url.channel()) else {
        tracing::debug!(
            "Skipping channel input with non-release token: {}",
            url.channel()
        );
        return Ok(None);
    };

    for candidate in token.future_candidates() {
        if exists(&url.probe_url(&candidate))? {
            tracing::debug!("Found published channel: {}", candidate);
            return Ok(Some(candidate));
        }
    }

    tracing::debug!("{} is already on the latest channel", url.channel());
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(uri: &str) -> ChannelTarballUrl {
        ChannelTarballUrl::parse(uri).unwrap_or_else(|| panic!("{uri} should parse"))
    }

    #[test]
    fn parses_canonical_channels_host() {
        let url = parse("https://channels.nixos.org/nixos-26.05/nixexprs.tar.xz");
        assert_eq!(url.scheme, "https");
        assert_eq!(url.host, "channels.nixos.org");
        assert_eq!(url.path_prefix, "");
        assert_eq!(url.channel, "nixos-26.05");
        assert_eq!(url.tail, "");
        assert_eq!(url.base(), "https://channels.nixos.org");
    }

    #[test]
    fn parses_legacy_nixos_org_channels_host() {
        let url = parse("https://nixos.org/channels/nixos-26.05/nixexprs.tar.xz");
        assert_eq!(url.host, "nixos.org");
        assert_eq!(url.path_prefix, "channels");
        assert_eq!(url.channel, "nixos-26.05");
        assert_eq!(url.base(), "https://nixos.org/channels");
    }

    #[test]
    fn unstable_channel_parses_as_a_url_but_never_bumps() {
        let url = parse("https://channels.nixos.org/nixos-unstable/nixexprs.tar.xz");
        assert_eq!(url.channel, "nixos-unstable");
        assert!(ChannelToken::parse("nixos-unstable").is_none());
        assert!(ChannelToken::parse("nixpkgs-unstable").is_none());
        assert!(ChannelToken::parse("nixos-unstable-small").is_none());

        let latest = find_latest_published_channel(&url, |_| {
            panic!("an unstable channel must not be probed")
        })
        .expect("unstable resolution should not error");
        assert_eq!(latest, None);
    }

    #[test]
    fn rejects_unknown_hosts_and_paths() {
        assert!(
            ChannelTarballUrl::parse("https://example.com/nixos-26.05/nixexprs.tar.xz").is_none()
        );
        assert!(
            ChannelTarballUrl::parse(
                "https://releases.nixos.org/nixos/26.05/nixos-26.05.6815.531670d871c0/nixexprs.tar.xz"
            )
            .is_none()
        );
        assert!(
            ChannelTarballUrl::parse("https://nixos.org/nixos-26.05/nixexprs.tar.xz").is_none()
        );
        assert!(
            ChannelTarballUrl::parse("https://channels.nixos.org/a/nixos-26.05/nixexprs.tar.xz")
                .is_none()
        );
        assert!(ChannelTarballUrl::parse("https://channels.nixos.org/nixos-26.05").is_none());
        assert!(
            ChannelTarballUrl::parse("https://channels.nixos.org/nixos-26.05/git-revision")
                .is_none()
        );
        assert!(
            ChannelTarballUrl::parse("https://channels.nixos.org/nixos-26.05/nixexprs.tar.bz2")
                .is_none()
        );
        assert!(ChannelTarballUrl::parse("channel:nixos-26.05").is_none());
        assert!(ChannelTarballUrl::parse("ftp://nixos.org/channels/x/nixexprs.tar.xz").is_none());
    }

    #[test]
    fn probe_url_matches_the_written_url() {
        let url = parse("https://channels.nixos.org/nixos-25.05/nixexprs.tar.xz");
        assert_eq!(
            url.probe_url("nixos-26.05"),
            "https://channels.nixos.org/nixos-26.05/nixexprs.tar.xz"
        );
        assert_eq!(
            url.probe_url("nixos-26.05"),
            url.with_channel("nixos-26.05")
        );
    }

    #[test]
    fn query_tail_survives_a_channel_bump() {
        let uri =
            "https://channels.nixos.org/nixos-26.05/nixexprs.tar.xz?narHash=sha256-Ab%2BCd/Ef0%3D";
        let url = parse(uri);
        assert_eq!(url.tail, "?narHash=sha256-Ab%2BCd/Ef0%3D");
        assert_eq!(url.with_channel(url.channel()), uri);
        assert_eq!(
            url.with_channel("nixos-26.11"),
            "https://channels.nixos.org/nixos-26.11/nixexprs.tar.xz?narHash=sha256-Ab%2BCd/Ef0%3D"
        );
        assert_eq!(
            url.probe_url("nixos-26.11"),
            "https://channels.nixos.org/nixos-26.11/nixexprs.tar.xz"
        );
    }

    #[test]
    fn token_parses_release_and_variant() {
        assert_eq!(
            ChannelToken::parse("nixos-26.05"),
            Some(ChannelToken {
                prefix: "nixos-".to_string(),
                version: (26, 5),
                suffix: String::new(),
            })
        );
        assert_eq!(
            ChannelToken::parse("nixos-26.05-small"),
            Some(ChannelToken {
                prefix: "nixos-".to_string(),
                version: (26, 5),
                suffix: "-small".to_string(),
            })
        );
        assert_eq!(
            ChannelToken::parse("nixpkgs-25.11-darwin"),
            Some(ChannelToken {
                prefix: "nixpkgs-".to_string(),
                version: (25, 11),
                suffix: "-darwin".to_string(),
            })
        );
    }

    #[test]
    fn token_rejects_non_release_shapes() {
        assert!(
            ChannelToken::parse("nixos-26.06").is_none(),
            "June is not a release month"
        );
        assert!(
            ChannelToken::parse("nixos-26.5").is_none(),
            "month must be two digits"
        );
        assert!(ChannelToken::parse("nixos-2026.05").is_none());
        assert!(
            ChannelToken::parse("nixos-19.05").is_none(),
            "year must be >= 20"
        );
        assert!(ChannelToken::parse("release-25.11").is_none());
        assert!(ChannelToken::parse("nix-darwin-25.11").is_none());
        assert!(ChannelToken::parse("nixos-26.05-tiny").is_none());
        assert!(ChannelToken::parse("26.05").is_none());
    }

    #[test]
    fn candidates_run_newest_first_and_only_forward() {
        let token = ChannelToken::parse("nixos-25.05").unwrap();
        let candidates = token.future_candidates();
        assert_eq!(candidates[0], "nixos-30.05");
        assert_eq!(candidates[candidates.len() - 1], "nixos-25.11");
        assert!(!candidates.contains(&"nixos-25.05".to_string()));
        assert!(!candidates.contains(&"nixos-24.11".to_string()));
    }

    #[test]
    fn variant_suffix_rides_along_on_every_candidate() {
        let token = ChannelToken::parse("nixpkgs-25.11-darwin").unwrap();
        let candidates = token.future_candidates();
        assert!(
            candidates.iter().all(|c| c.ends_with("-darwin")),
            "variant suffix must be preserved: {candidates:?}"
        );
        assert_eq!(candidates[candidates.len() - 1], "nixpkgs-26.05-darwin");
    }

    #[test]
    fn resolves_to_the_newest_published_channel() {
        let url = parse("https://channels.nixos.org/nixos-25.05/nixexprs.tar.xz");
        let published = ["nixos-25.11", "nixos-26.05"];
        let mut probed = Vec::new();

        let latest = find_latest_published_channel(&url, |probe| {
            probed.push(probe.to_string());
            Ok(published
                .iter()
                .any(|c| probe == format!("https://channels.nixos.org/{c}/nixexprs.tar.xz")))
        })
        .expect("resolution should succeed");

        assert_eq!(latest, Some("nixos-26.05".to_string()));
        assert!(
            !probed.contains(&"https://channels.nixos.org/nixos-25.11/nixexprs.tar.xz".to_string())
        );
        assert!(
            probed.iter().all(|p| p.ends_with("/nixexprs.tar.xz")),
            "probes must target the tarball: {probed:?}"
        );
        assert_eq!(
            url.with_channel(&latest.unwrap()),
            "https://channels.nixos.org/nixos-26.05/nixexprs.tar.xz"
        );
    }

    #[test]
    fn probes_the_host_the_input_already_uses() {
        let url = parse("https://nixos.org/channels/nixos-25.05/nixexprs.tar.xz");
        let latest = find_latest_published_channel(&url, |probe| {
            Ok(probe == "https://nixos.org/channels/nixos-25.11/nixexprs.tar.xz")
        })
        .expect("resolution should succeed");
        assert_eq!(latest, Some("nixos-25.11".to_string()));
        assert_eq!(
            url.with_channel(&latest.unwrap()),
            "https://nixos.org/channels/nixos-25.11/nixexprs.tar.xz"
        );
    }

    #[test]
    fn nothing_newer_published_means_no_change() {
        let url = parse("https://channels.nixos.org/nixos-26.05/nixexprs.tar.xz");
        let latest =
            find_latest_published_channel(&url, |_| Ok(false)).expect("resolution should succeed");
        assert_eq!(latest, None);
    }

    #[test]
    fn transient_probe_failure_is_not_reported_as_up_to_date() {
        let url = parse("https://channels.nixos.org/nixos-25.05/nixexprs.tar.xz");
        let result = find_latest_published_channel(&url, |probe| {
            Err(ApiError::Timeout {
                url: probe.to_string(),
                source: Box::new(ureq::Error::HostNotFound),
            })
        });
        assert!(
            result.is_err(),
            "a probe failure must propagate, not resolve to None"
        );
    }
}
