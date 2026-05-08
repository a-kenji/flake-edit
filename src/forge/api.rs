use std::collections::HashMap;
use std::process::Command;

use semver::Version;
use serde::Deserialize;
use thiserror::Error;
use ureq::Agent;

use super::version::parse_ref;

#[derive(Error, Debug)]
pub enum ApiError {
    #[error("HTTP request failed: {0}")]
    HttpError(#[from] ureq::Error),

    #[error("JSON parsing failed: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("UTF-8 conversion failed: {0}")]
    Utf8Error(#[from] std::string::FromUtf8Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("No tags found for repository")]
    NoTagsFound,

    #[error("Invalid domain or repository: {0}")]
    InvalidInput(String),
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
        Self {
            agent: Agent::new_with_defaults(),
        }
    }
}

impl HttpClient {
    fn get(&self, url: &str, headers: &Headers) -> Result<String, ApiError> {
        let body = self
            .build(url, headers)
            .call()?
            .body_mut()
            .read_to_string()?;
        Ok(body)
    }

    /// Check if a URL returns a successful (2xx) response
    fn head_ok(&self, url: &str, headers: &Headers) -> bool {
        self.build(url, headers).call().is_ok()
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
    fn branch_exists(&self, owner: &str, repo: &str, branch: &str) -> bool;
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
            let page_tags = serde_json::from_str::<IntermediaryTags>(&body)?;
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
            let page_branches = serde_json::from_str::<IntermediaryBranches>(&body)?;
            tracing::debug!("Got {} branches on page {}", page_branches.0.len(), page);
            Ok(page_branches.0)
        })?;
        tracing::debug!("Total branches fetched: {}", branches.len());
        Ok(IntermediaryBranches(branches).into())
    }

    fn branch_exists(&self, owner: &str, repo: &str, branch: &str) -> bool {
        let headers = Headers::for_domain("github.com");
        let url = format!("https://api.github.com/repos/{owner}/{repo}/branches/{branch}");
        self.http.head_ok(&url, &headers)
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

        Err(ApiError::InvalidInput("Could not fetch branches".into()))
    }

    fn branch_exists(&self, owner: &str, repo: &str, branch: &str) -> bool {
        let headers = Headers::for_domain(&self.domain);
        for scheme in ["https", "http"] {
            let url = format!(
                "{}://{}/api/v1/repos/{}/{}/branches/{}",
                scheme, self.domain, owner, repo, branch
            );
            if self.http.head_ok(&url, &headers) {
                return true;
            }
        }
        false
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
pub struct IntermediaryTags(Vec<IntermediaryTag>);

#[derive(Deserialize, Debug)]
pub struct IntermediaryBranches(Vec<IntermediaryBranch>);

#[derive(Deserialize, Debug)]
pub struct IntermediaryBranch {
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
pub struct IntermediaryTag {
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
/// Much more efficient for repos with many branches (like nixpkgs).
pub fn branch_exists(owner: &str, repo: &str, branch: &str, domain: Option<&str>) -> bool {
    forge_for(domain).branch_exists(owner, repo, branch)
}

/// Check multiple branches and return which ones exist.
/// More efficient than get_branches for known candidate branches.
pub fn filter_existing_branches(
    owner: &str,
    repo: &str,
    candidates: &[String],
    domain: Option<&str>,
) -> Vec<String> {
    let forge = forge_for(domain);
    candidates
        .iter()
        .filter(|branch| forge.branch_exists(owner, repo, branch))
        .cloned()
        .collect()
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
}
