use std::collections::HashMap;
use std::process::Command;
use std::time::Duration;

use semver::Version;
use serde::Deserialize;
use thiserror::Error;
use ureq::Agent;

use super::version::parse_ref;

type SourceError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// Errors from talking to a forge over HTTP.
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum ApiError {
    /// Request hit a configured timeout (connect, recv-response,
    /// or recv-body).
    #[error("request to {url} timed out")]
    Timeout {
        url: String,
        #[source]
        source: SourceError,
    },

    /// Could not establish a connection. Covers DNS resolution
    /// failure (`HostNotFound`), TCP connect refusal, TLS handshake
    /// failure, and underlying IO errors below the HTTP layer.
    #[error("could not reach {url}")]
    ConnectFailed {
        url: String,
        #[source]
        source: SourceError,
    },

    #[error("{url} not found (HTTP 404)")]
    NotFound { url: String },

    /// Non-404 HTTP error response (other 4xx, all 5xx). Almost
    /// always a transient server-side condition or a misformed
    /// request, distinct from "the resource is gone".
    #[error("{url} returned HTTP {status}")]
    HttpStatus { url: String, status: u16 },

    /// Failed to parse the JSON response body returned by the forge.
    #[error("failed to parse JSON response from {url}")]
    Json {
        url: String,
        #[source]
        source: serde_json::Error,
    },

    /// HTTP error not classified above. Reached only if ureq grows
    /// a new variant or a bespoke connector chain returns one of the
    /// rarer existing variants (`Tls`, `Protocol`, ...). Treat as a
    /// transient failure.
    #[error("unexpected HTTP error for {url}")]
    Other {
        url: String,
        #[source]
        source: SourceError,
    },

    /// The forge returned no tags for the repository.
    #[error("no tags found for repository")]
    NoTagsFound,

    /// Branch listing exhausted retries (both schemes, all pages) without
    /// returning usable data.
    #[error("no branches found for repository")]
    NoBranchesFound,
}

/// Classify a `ureq::Error` from establishing the request (DNS,
/// connect, send, recv-response, status code) into the domain
/// `ApiError`. Hand-written instead of `#[from]` so a future ureq
/// variant cannot silently disappear into a catch-all; it falls
/// into `Other` deliberately.
fn classify_ureq(err: ureq::Error, url: &str) -> ApiError {
    let url = url.to_string();
    match err {
        ureq::Error::StatusCode(404) => ApiError::NotFound { url },
        ureq::Error::StatusCode(status) => ApiError::HttpStatus { url, status },
        ureq::Error::Timeout(_) => ApiError::Timeout {
            url,
            source: Box::new(err),
        },
        ureq::Error::HostNotFound | ureq::Error::ConnectionFailed | ureq::Error::Io(_) => {
            ApiError::ConnectFailed {
                url,
                source: Box::new(err),
            }
        }
        _ => ApiError::Other {
            url,
            source: Box::new(err),
        },
    }
}

/// Classify a `ureq::Error` from reading the response body off a
/// connection that already succeeded. Distinct from `classify_ureq`
/// because here an `Io(_)` is a mid-stream drop, not a connect
/// failure; calling it `ConnectFailed` would tell the user we never
/// reached the forge when in fact the peer hung up halfway through.
fn classify_body_read(err: ureq::Error, url: &str) -> ApiError {
    let url = url.to_string();
    match err {
        ureq::Error::Timeout(_) => ApiError::Timeout {
            url,
            source: Box::new(err),
        },
        _ => ApiError::Other {
            url,
            source: Box::new(err),
        },
    }
}

/// Headers for HTTP requests
#[derive(Clone, Default)]
struct Headers {
    user_agent: Option<String>,
    authorization: Option<String>,
}

impl Headers {
    fn base() -> Self {
        Self {
            user_agent: Some("flake-edit".to_string()),
            authorization: None,
        }
    }

    /// Headers with optional Bearer token authentication for the given domain.
    fn for_domain(domain: &str) -> Self {
        let mut headers = Self::base();
        if let Some(token) = get_forge_token(domain) {
            tracing::debug!("Found token for {}", domain);
            headers.authorization = Some(format!("Bearer {token}"));
        }
        headers
    }
}

struct HttpClient {
    agent: Agent,
}

impl Default for HttpClient {
    fn default() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .timeout_recv_body(Some(Duration::from_secs(30)))
            .build();
        Self {
            agent: Agent::new_with_config(config),
        }
    }
}

impl HttpClient {
    fn get(&self, url: &str, headers: &Headers) -> Result<String, ApiError> {
        let body = self
            .build(url, headers)
            .call()
            .map_err(|e| classify_ureq(e, url))?
            .body_mut()
            .read_to_string()
            .map_err(|e| classify_body_read(e, url))?;
        Ok(body)
    }

    /// Probe a URL and report whether the resource exists.
    ///
    /// `Ok(true)` for any 2xx response, `Ok(false)` for HTTP 404,
    /// `Err(_)` for anything else (timeout, connect failure, 5xx,
    /// ...). The caller must not collapse these three into a bool:
    /// "branch does not exist" and "could not reach the forge" are
    /// different answers and the user needs to see the latter.
    fn head_status(&self, url: &str, headers: &Headers) -> Result<bool, ApiError> {
        match self.build(url, headers).call() {
            Ok(_) => Ok(true),
            Err(e) => match classify_ureq(e, url) {
                ApiError::NotFound { .. } => Ok(false),
                other => Err(other),
            },
        }
    }

    fn build(
        &self,
        url: &str,
        headers: &Headers,
    ) -> ureq::RequestBuilder<ureq::typestate::WithoutBody> {
        let mut request = self.agent.get(url);
        if let Some(ref ua) = headers.user_agent {
            request = request.header("User-Agent", ua);
        }
        if let Some(ref auth) = headers.authorization {
            request = request.header("Authorization", auth);
        }
        request
    }
}

pub(crate) trait Forge {
    fn list_tags(&self, owner: &str, repo: &str) -> Result<Tags, ApiError>;
    fn list_branches(&self, owner: &str, repo: &str) -> Result<Branches, ApiError>;
    fn branch_exists(&self, owner: &str, repo: &str, branch: &str) -> Result<bool, ApiError>;
}

struct GitHub {
    http: HttpClient,
}

impl GitHub {
    /// Hard cap on paginated listing requests. Shared by `list_tags`
    /// and `list_branches` so the two paths cannot drift apart.
    const MAX_PAGES: u32 = 20;
    /// GitHub's maximum page size for list endpoints.
    const PER_PAGE: usize = 100;

    fn new() -> Self {
        Self {
            http: HttpClient::default(),
        }
    }
}

/// Drive a paginated listing endpoint until a short page or the
/// safety cap is hit, accumulating items in order.
///
/// `fetch` receives a 1-based page number and returns the items on
/// that page. Iteration stops when `fetch` yields fewer than
/// `per_page` items (real APIs signal "no more" with a short page)
/// or when `page` reaches `max_pages` (safety cap against runaway
/// loops on a misbehaving endpoint that always returns full pages).
fn paginated<T, F>(per_page: usize, max_pages: u32, mut fetch: F) -> Result<Vec<T>, ApiError>
where
    F: FnMut(u32) -> Result<Vec<T>, ApiError>,
{
    let mut all = Vec::new();
    let mut page: u32 = 1;
    loop {
        let items = fetch(page)?;
        let count = items.len();
        all.extend(items);
        if count < per_page || page >= max_pages {
            break;
        }
        page += 1;
    }
    Ok(all)
}

impl Forge for GitHub {
    fn list_tags(&self, owner: &str, repo: &str) -> Result<Tags, ApiError> {
        let headers = Headers::for_domain("github.com");
        let tags = paginated(Self::PER_PAGE, Self::MAX_PAGES, |page| {
            let url = format!(
                "https://api.github.com/repos/{}/{}/tags?per_page={}&page={}",
                owner,
                repo,
                Self::PER_PAGE,
                page
            );
            tracing::debug!("Fetching tags page {}: {}", page, url);
            let body = self.http.get(&url, &headers)?;
            let page_tags = serde_json::from_str::<IntermediaryTags>(&body).map_err(|source| {
                ApiError::Json {
                    url: url.clone(),
                    source,
                }
            })?;
            tracing::debug!("Got {} tags on page {}", page_tags.0.len(), page);
            Ok(page_tags.0)
        })?;
        tracing::debug!("Total tags fetched: {}", tags.len());
        Ok(IntermediaryTags(tags).into())
    }

    fn list_branches(&self, owner: &str, repo: &str) -> Result<Branches, ApiError> {
        let headers = Headers::for_domain("github.com");
        let branches = paginated(Self::PER_PAGE, Self::MAX_PAGES, |page| {
            let url = format!(
                "https://api.github.com/repos/{}/{}/branches?per_page={}&page={}",
                owner,
                repo,
                Self::PER_PAGE,
                page
            );
            tracing::debug!("Fetching branches page {}: {}", page, url);
            let body = self.http.get(&url, &headers)?;
            let page_branches =
                serde_json::from_str::<IntermediaryBranches>(&body).map_err(|source| {
                    ApiError::Json {
                        url: url.clone(),
                        source,
                    }
                })?;
            tracing::debug!("Got {} branches on page {}", page_branches.0.len(), page);
            Ok(page_branches.0)
        })?;
        tracing::debug!("Total branches fetched: {}", branches.len());
        Ok(IntermediaryBranches(branches).into())
    }

    fn branch_exists(&self, owner: &str, repo: &str, branch: &str) -> Result<bool, ApiError> {
        let headers = Headers::for_domain("github.com");
        let url = format!("https://api.github.com/repos/{owner}/{repo}/branches/{branch}");
        self.http.head_status(&url, &headers)
    }
}

struct Gitea {
    http: HttpClient,
    domain: String,
}

impl Gitea {
    fn new(domain: String) -> Self {
        Self {
            http: HttpClient::default(),
            domain,
        }
    }
}

impl Forge for Gitea {
    fn list_tags(&self, owner: &str, repo: &str) -> Result<Tags, ApiError> {
        let headers = Headers::for_domain(&self.domain);

        // Try HTTPS first, then HTTP
        for scheme in ["https", "http"] {
            let url = format!(
                "{}://{}/api/v1/repos/{}/{}/tags",
                scheme, self.domain, owner, repo
            );
            tracing::debug!("Trying Gitea tags endpoint: {}", url);

            if let Ok(body) = self.http.get(&url, &headers) {
                tracing::debug!("Body from Gitea API: {body}");
                if let Ok(tags) = serde_json::from_str::<IntermediaryTags>(&body) {
                    return Ok(tags.into());
                }
            }
        }

        Err(ApiError::NoTagsFound)
    }

    fn list_branches(&self, owner: &str, repo: &str) -> Result<Branches, ApiError> {
        let headers = Headers::for_domain(&self.domain);
        let mut all_branches = Vec::new();
        let mut page = 1;
        const MAX_PAGES: u32 = 20;

        // Try HTTPS first, then HTTP
        for scheme in ["https", "http"] {
            loop {
                let url = format!(
                    "{}://{}/api/v1/repos/{}/{}/branches?limit=50&page={}",
                    scheme, self.domain, owner, repo, page
                );
                tracing::debug!("Trying Gitea branches endpoint: {}", url);

                match self.http.get(&url, &headers) {
                    Ok(body) => {
                        tracing::debug!("Body from Gitea API: {body}");
                        match serde_json::from_str::<IntermediaryBranches>(&body) {
                            Ok(page_branches) => {
                                let count = page_branches.0.len();
                                all_branches.extend(page_branches.0);

                                if count < 50 || page >= MAX_PAGES {
                                    return Ok(IntermediaryBranches(all_branches).into());
                                }
                                page += 1;
                            }
                            Err(_) => break, // Try next scheme
                        }
                    }
                    Err(_) => break, // Try next scheme
                }
            }

            if !all_branches.is_empty() {
                return Ok(IntermediaryBranches(all_branches).into());
            }
            page = 1; // Reset for next scheme
        }

        Err(ApiError::NoBranchesFound)
    }

    fn branch_exists(&self, owner: &str, repo: &str, branch: &str) -> Result<bool, ApiError> {
        let headers = Headers::for_domain(&self.domain);
        // Probe https first, fall back to http for misconfigured
        // self-hosted instances. The first scheme to give a
        // definitive answer (2xx or 404) wins; http is tried only
        // when https errored out before reaching the application
        // layer.
        let mut last_err: Option<ApiError> = None;
        for scheme in ["https", "http"] {
            let url = format!(
                "{}://{}/api/v1/repos/{}/{}/branches/{}",
                scheme, self.domain, owner, repo, branch
            );
            match self.http.head_status(&url, &headers) {
                Ok(answer) => return Ok(answer),
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.expect("at least one scheme was attempted"))
    }
}

/// Hard-coded [`GitHub`] for `github.com`. Every other domain gets
/// [`Gitea`], since self-hosted instances almost universally expose
/// `/api/v1/repos/...` even when they do not advertise themselves as
/// Gitea or Forgejo.
pub(crate) fn forge_for(domain: Option<&str>) -> Box<dyn Forge> {
    let domain = domain.unwrap_or("github.com");
    if domain == "github.com" {
        return Box::new(GitHub::new());
    }
    Box::new(Gitea::new(domain.to_string()))
}

#[derive(Deserialize, Debug)]
pub(crate) struct IntermediaryTags(Vec<IntermediaryTag>);

#[derive(Deserialize, Debug)]
pub(crate) struct IntermediaryBranches(Vec<IntermediaryBranch>);

#[derive(Deserialize, Debug)]
pub(crate) struct IntermediaryBranch {
    name: String,
}

#[derive(Debug, Default)]
pub struct Branches {
    pub names: Vec<String>,
}

#[derive(Debug)]
pub struct Tags {
    versions: Vec<TagVersion>,
}

impl Tags {
    pub fn get_latest_tag(&mut self) -> Option<String> {
        self.sort();
        self.versions.last().map(|tag| tag.original.clone())
    }
    pub fn sort(&mut self) {
        self.versions
            .sort_by(|a, b| a.version.cmp_precedence(&b.version));
    }
}

#[derive(Deserialize, Debug)]
pub(crate) struct IntermediaryTag {
    name: String,
}

#[derive(Debug)]
struct TagVersion {
    version: Version,
    original: String,
}

pub fn get_tags(owner: &str, repo: &str, domain: Option<&str>) -> Result<Tags, ApiError> {
    forge_for(domain).list_tags(owner, repo)
}

pub fn get_branches(owner: &str, repo: &str, domain: Option<&str>) -> Result<Branches, ApiError> {
    forge_for(domain).list_branches(owner, repo)
}

/// Check if a specific branch exists without listing all branches.
///
/// Much more efficient for repos with many branches (like nixpkgs).
/// `Ok(true)` for an existing branch, `Ok(false)` for HTTP 404,
/// `Err(_)` for any transient failure (timeout, DNS, 5xx, ...). The
/// caller must propagate the error rather than collapse it into
/// "the branch does not exist".
pub fn branch_exists(
    owner: &str,
    repo: &str,
    branch: &str,
    domain: Option<&str>,
) -> Result<bool, ApiError> {
    forge_for(domain).branch_exists(owner, repo, branch)
}

#[derive(Deserialize, Debug, Clone)]
struct NixConfig {
    #[serde(rename = "access-tokens")]
    access_tokens: Option<AccessTokens>,
}

impl NixConfig {
    fn forge_token(&self, domain: &str) -> Option<String> {
        self.access_tokens.as_ref()?.value.get(domain).cloned()
    }
}

#[derive(Deserialize, Debug, Clone)]
struct AccessTokens {
    value: HashMap<String, String>,
}

fn get_forge_token(domain: &str) -> Option<String> {
    // Try to get token from nix config
    if let Ok(output) = Command::new("nix")
        .arg("config")
        .arg("show")
        .arg("--json")
        .output()
        && let Ok(stdout) = String::from_utf8(output.stdout)
        && let Ok(config) = serde_json::from_str::<NixConfig>(&stdout)
        && let Some(token) = config.forge_token(domain)
    {
        return Some(token);
    }

    // Fallback to environment variables
    if let Ok(token) = std::env::var("GITEA_TOKEN") {
        return Some(token);
    }
    if let Ok(token) = std::env::var("FORGEJO_TOKEN") {
        return Some(token);
    }
    if domain == "github.com"
        && let Ok(token) = std::env::var("GITHUB_TOKEN")
    {
        return Some(token);
    }

    None
}

impl From<IntermediaryTags> for Tags {
    fn from(value: IntermediaryTags) -> Self {
        let mut versions = vec![];
        for itag in value.0 {
            let parsed = parse_ref(&itag.name, false);
            let normalized = parsed.normalized_for_semver;
            match Version::parse(&normalized) {
                Ok(semver) => {
                    versions.push(TagVersion {
                        version: semver,
                        original: parsed.original_ref,
                    });
                }
                Err(e) => {
                    tracing::error!("Could not parse version {:?}", e);
                }
            }
        }
        Tags { versions }
    }
}

impl From<IntermediaryBranches> for Branches {
    fn from(value: IntermediaryBranches) -> Self {
        Branches {
            names: value.0.into_iter().map(|b| b.name).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://api.github.com/repos/foo/bar/branches/baz";

    #[test]
    fn paginated_accumulates_until_short_page() {
        let pages: Vec<Vec<u32>> = vec![(0..100).collect(), (100..105).collect()];
        let mut calls = 0u32;
        let result = paginated::<u32, _>(100, GitHub::MAX_PAGES, |page| {
            calls += 1;
            Ok(pages[(page - 1) as usize].clone())
        })
        .unwrap();
        assert_eq!(calls, 2, "stops after the short page, no third request");
        assert_eq!(result.len(), 105);
        assert_eq!(result.first().copied(), Some(0));
        assert_eq!(result.last().copied(), Some(104));
    }

    #[test]
    fn paginated_caps_at_max_pages() {
        let mut calls = 0u32;
        let result = paginated::<u32, _>(2, 3, |_| {
            calls += 1;
            Ok(vec![1, 2])
        })
        .unwrap();
        assert_eq!(calls, 3, "safety cap halts the loop on always-full pages");
        assert_eq!(result.len(), 6);
    }

    #[test]
    fn classify_404_is_not_found() {
        let err = classify_ureq(ureq::Error::StatusCode(404), URL);
        match err {
            ApiError::NotFound { url } => assert_eq!(url, URL),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn classify_500_is_http_status() {
        let err = classify_ureq(ureq::Error::StatusCode(503), URL);
        match err {
            ApiError::HttpStatus { url, status } => {
                assert_eq!(url, URL);
                assert_eq!(status, 503);
            }
            other => panic!("expected HttpStatus, got {other:?}"),
        }
    }

    #[test]
    fn body_read_io_is_not_connect_failed() {
        let io = std::io::Error::other("peer closed");
        let err = classify_body_read(ureq::Error::Io(io), URL);
        match err {
            ApiError::Other { url, .. } => assert_eq!(url, URL),
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[test]
    fn tags_parsing_with_refs_tags_prefix() {
        let json = r#"[
            {"name": "refs/tags/v1.0.0"},
            {"name": "refs/tags/v2.0.0"},
            {"name": "refs/tags/v1.5.0"}
        ]"#;

        let intermediary: IntermediaryTags = serde_json::from_str(json).unwrap();
        let mut tags: Tags = intermediary.into();

        assert_eq!(tags.get_latest_tag(), Some("refs/tags/v2.0.0".to_string()));
    }

    #[test]
    fn tags_parsing_with_short_versions() {
        let json = r#"[
            {"name": "v1"},
            {"name": "v1.1"}
        ]"#;

        let intermediary: IntermediaryTags = serde_json::from_str(json).unwrap();
        let mut tags: Tags = intermediary.into();

        assert_eq!(tags.get_latest_tag(), Some("v1.1".to_string()));
    }

    #[test]
    fn tags_parsing_without_prefix() {
        let json = r#"[
            {"name": "1.0.0"},
            {"name": "2.0.0"},
            {"name": "1.5.0"}
        ]"#;

        let intermediary: IntermediaryTags = serde_json::from_str(json).unwrap();
        let mut tags: Tags = intermediary.into();

        assert_eq!(tags.get_latest_tag(), Some("2.0.0".to_string()));
    }

    #[test]
    fn tags_parsing_with_dash_prefix() {
        let json = r#"[
            {"name": "release-1.0.0"},
            {"name": "release-2.0.0"},
            {"name": "release-1.5.0"}
        ]"#;

        let intermediary: IntermediaryTags = serde_json::from_str(json).unwrap();
        let mut tags: Tags = intermediary.into();

        assert_eq!(tags.get_latest_tag(), Some("release-2.0.0".to_string()));
    }

    #[test]
    fn tags_parsing_mixed_valid_invalid() {
        let json = r#"[
            {"name": "v1.0.0"},
            {"name": "v2.0.0"},
            {"name": "invalid-tag"},
            {"name": "v1.5.0"}
        ]"#;

        let intermediary: IntermediaryTags = serde_json::from_str(json).unwrap();
        let mut tags: Tags = intermediary.into();

        assert_eq!(tags.get_latest_tag(), Some("v2.0.0".to_string()));
    }

    #[test]
    fn tags_parsing_empty() {
        let json = r#"[]"#;

        let intermediary: IntermediaryTags = serde_json::from_str(json).unwrap();
        let mut tags: Tags = intermediary.into();

        assert_eq!(tags.get_latest_tag(), None);
    }

    #[test]
    fn tags_parsing_drops_prerelease() {
        let json = r#"[
            {"name": "v1.0.0"},
            {"name": "v2.0.0-beta.1"},
            {"name": "v1.5.0"}
        ]"#;

        let intermediary: IntermediaryTags = serde_json::from_str(json).unwrap();
        let mut tags: Tags = intermediary.into();

        // `parse_ref` strips everything after the first `-`, leaving
        // a non-numeric "beta.1" which `Version::parse` rejects, so
        // the prerelease tag is filtered out and the latest stable
        // wins.
        assert_eq!(tags.get_latest_tag(), Some("v1.5.0".to_string()));
    }

    #[test]
    fn tags_parsing_combined_prefixes() {
        let json = r#"[
            {"name": "refs/tags/v1.0.0"},
            {"name": "refs/tags/v2.0.0"}
        ]"#;

        let intermediary: IntermediaryTags = serde_json::from_str(json).unwrap();
        let mut tags: Tags = intermediary.into();

        assert_eq!(tags.get_latest_tag(), Some("refs/tags/v2.0.0".to_string()));
    }

    #[test]
    fn tags_sort_by_semver_not_lex() {
        let json = r#"[
            {"name": "v10.0.0"},
            {"name": "v2.0.0"},
            {"name": "v1.0.0"}
        ]"#;

        let intermediary: IntermediaryTags = serde_json::from_str(json).unwrap();
        let mut tags: Tags = intermediary.into();

        assert_eq!(tags.get_latest_tag(), Some("v10.0.0".to_string()));
    }

    #[test]
    fn http_client_has_explicit_timeouts() {
        // Without timeouts a hung TCP connect blocks the whole CLI.
        // The exact values are tunable but they must not be unset.
        let client = HttpClient::default();
        let timeouts = client.agent.config().timeouts();
        assert!(
            timeouts.connect.is_some(),
            "connect timeout must be set on the HTTP agent"
        );
        assert!(
            timeouts.recv_response.is_some(),
            "recv_response timeout must be set on the HTTP agent"
        );
    }
}
