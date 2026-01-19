use std::collections::HashMap;
use std::process::Command;

use semver::Version;
use serde::Deserialize;
use thiserror::Error;
use ureq::Agent;

use crate::version::parse_ref;

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

pub struct ForgeClient {
    agent: Agent,
}

impl Default for ForgeClient {
    fn default() -> Self {
        Self {
            agent: Agent::new_with_defaults(),
        }
    }
}

impl ForgeClient {
    fn get(&self, url: &str, headers: &Headers) -> Result<String, ApiError> {
        let mut request = self.agent.get(url);
        if let Some(ref ua) = headers.user_agent {
            request = request.header("User-Agent", ua);
        }
        if let Some(ref auth) = headers.authorization {
            request = request.header("Authorization", auth);
        }
        let body = request.call()?.body_mut().read_to_string()?;
        Ok(body)
    }

    /// Check if a URL returns a successful (2xx) response
    fn head_ok(&self, url: &str, headers: &Headers) -> bool {
        let mut request = self.agent.get(url);
        if let Some(ref ua) = headers.user_agent {
            request = request.header("User-Agent", ua);
        }
        if let Some(ref auth) = headers.authorization {
            request = request.header("Authorization", auth);
        }
        request.call().is_ok()
    }

    fn base_headers() -> Headers {
        Headers {
            user_agent: Some("flake-edit".to_string()),
            authorization: None,
        }
    }

    /// Create headers with optional Bearer token authentication for the given domain.
    fn auth_headers(domain: &str) -> Headers {
        let mut headers = Self::base_headers();
        if let Some(token) = get_forge_token(domain) {
            tracing::debug!("Found token for {}", domain);
            headers.authorization = Some(format!("Bearer {token}"));
        }
        headers
    }

    pub fn detect_forge_type(&self, domain: &str) -> ForgeType {
        if domain == "github.com" {
            return ForgeType::GitHub;
        }

        tracing::debug!("Attempting to detect forge type for domain: {}", domain);
        let headers = Self::base_headers();

        // Try both HTTPS and HTTP for each endpoint
        for scheme in ["https", "http"] {
            // Try Forgejo version endpoint first
            let forgejo_url = format!("{}://{}/api/forgejo/v1/version", scheme, domain);
            tracing::debug!("Trying Forgejo endpoint: {}", forgejo_url);
            if let Ok(text) = self.get(&forgejo_url, &headers) {
                tracing::debug!("Forgejo endpoint response body: {}", text);
                if let Some(forge_type) = parse_forge_version(&text) {
                    tracing::info!("Detected Forgejo/Gitea at {}", domain);
                    return forge_type;
                }
            }

            // Try Gitea version endpoint
            let gitea_url = format!("{}://{}/api/v1/version", scheme, domain);
            tracing::debug!("Trying Gitea endpoint: {}", gitea_url);
            if let Ok(text) = self.get(&gitea_url, &headers) {
                tracing::debug!("Gitea endpoint response body: {}", text);
                if let Some(forge_type) = parse_forge_version(&text) {
                    tracing::info!("Detected Forgejo/Gitea at {}", domain);
                    return forge_type;
                }
                // Plain Gitea just has a version number without +gitea or +forgejo
                if serde_json::from_str::<ForgeVersion>(&text).is_ok() {
                    tracing::info!("Detected Gitea at {}", domain);
                    return ForgeType::Gitea;
                }
            }
        }

        tracing::warn!(
            "Could not detect forge type for {}, will try GitHub API as fallback",
            domain
        );
        ForgeType::Unknown
    }

    pub fn query_github_tags(&self, repo: &str, owner: &str) -> Result<IntermediaryTags, ApiError> {
        let headers = Self::auth_headers("github.com");
        let body = self.get(
            &format!("https://api.github.com/repos/{}/{}/tags", owner, repo),
            &headers,
        )?;

        tracing::debug!("Body from api: {body}");
        let tags = serde_json::from_str::<IntermediaryTags>(&body)?;
        Ok(tags)
    }

    /// Check if a specific branch exists (returns true/false, no error on 404)
    pub fn branch_exists_github(&self, repo: &str, owner: &str, branch: &str) -> bool {
        let headers = Self::auth_headers("github.com");
        let url = format!(
            "https://api.github.com/repos/{}/{}/branches/{}",
            owner, repo, branch
        );

        self.head_ok(&url, &headers)
    }

    /// Check if a specific branch exists on Gitea/Forgejo
    pub fn branch_exists_gitea(&self, repo: &str, owner: &str, domain: &str, branch: &str) -> bool {
        let headers = Self::auth_headers(domain);
        for scheme in ["https", "http"] {
            let url = format!(
                "{}://{}/api/v1/repos/{}/{}/branches/{}",
                scheme, domain, owner, repo, branch
            );
            if self.head_ok(&url, &headers) {
                return true;
            }
        }
        false
    }

    pub fn query_gitea_tags(
        &self,
        repo: &str,
        owner: &str,
        domain: &str,
    ) -> Result<IntermediaryTags, ApiError> {
        let headers = Self::auth_headers(domain);

        // Try HTTPS first, then HTTP
        for scheme in ["https", "http"] {
            let url = format!(
                "{}://{}/api/v1/repos/{}/{}/tags",
                scheme, domain, owner, repo
            );
            tracing::debug!("Trying Gitea tags endpoint: {}", url);

            if let Ok(body) = self.get(&url, &headers) {
                tracing::debug!("Body from Gitea API: {body}");
                if let Ok(tags) = serde_json::from_str::<IntermediaryTags>(&body) {
                    return Ok(tags);
                }
            }
        }

        Err(ApiError::NoTagsFound)
    }

    pub fn query_github_branches(
        &self,
        repo: &str,
        owner: &str,
    ) -> Result<IntermediaryBranches, ApiError> {
        let headers = Self::auth_headers("github.com");

        let mut all_branches = Vec::new();
        let mut page = 1;
        const MAX_PAGES: u32 = 20; // Safety limit to avoid infinite loops

        loop {
            let url = format!(
                "https://api.github.com/repos/{}/{}/branches?per_page=100&page={}",
                owner, repo, page
            );
            tracing::debug!("Fetching branches page {}: {}", page, url);

            let body = self.get(&url, &headers)?;
            let page_branches = serde_json::from_str::<IntermediaryBranches>(&body)?;

            let count = page_branches.0.len();
            tracing::debug!("Got {} branches on page {}", count, page);

            all_branches.extend(page_branches.0);

            // Stop if we got fewer than 100 (last page) or hit max pages
            if count < 100 || page >= MAX_PAGES {
                break;
            }

            page += 1;
        }

        tracing::debug!("Total branches fetched: {}", all_branches.len());
        Ok(IntermediaryBranches(all_branches))
    }

    pub fn query_gitea_branches(
        &self,
        repo: &str,
        owner: &str,
        domain: &str,
    ) -> Result<IntermediaryBranches, ApiError> {
        let headers = Self::auth_headers(domain);

        let mut all_branches = Vec::new();
        let mut page = 1;
        const MAX_PAGES: u32 = 20;

        // Try HTTPS first, then HTTP
        for scheme in ["https", "http"] {
            loop {
                let url = format!(
                    "{}://{}/api/v1/repos/{}/{}/branches?limit=50&page={}",
                    scheme, domain, owner, repo, page
                );
                tracing::debug!("Trying Gitea branches endpoint: {}", url);

                match self.get(&url, &headers) {
                    Ok(body) => {
                        tracing::debug!("Body from Gitea API: {body}");
                        match serde_json::from_str::<IntermediaryBranches>(&body) {
                            Ok(page_branches) => {
                                let count = page_branches.0.len();
                                all_branches.extend(page_branches.0);

                                if count < 50 || page >= MAX_PAGES {
                                    return Ok(IntermediaryBranches(all_branches));
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
                return Ok(IntermediaryBranches(all_branches));
            }
            page = 1; // Reset for next scheme
        }

        Err(ApiError::InvalidInput("Could not fetch branches".into()))
    }
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

#[derive(Deserialize, Debug)]
struct ForgeVersion {
    version: String,
}

#[derive(Debug, PartialEq)]
pub enum ForgeType {
    GitHub,
    Gitea, // Covers both Gitea and Forgejo
    Unknown,
}

fn parse_forge_version(json: &str) -> Option<ForgeType> {
    serde_json::from_str::<ForgeVersion>(json)
        .ok()
        .and_then(|v| {
            if v.version.contains("+forgejo") || v.version.contains("+gitea") {
                Some(ForgeType::Gitea)
            } else {
                None
            }
        })
}

// Test helpers are always available but not documented
#[doc(hidden)]
pub mod test_helpers {
    use super::*;

    pub fn parse_forge_version_test(json: &str) -> Option<ForgeType> {
        parse_forge_version(json)
    }
}

pub fn get_tags(repo: &str, owner: &str, domain: Option<&str>) -> Result<Tags, ApiError> {
    let domain = domain.unwrap_or("github.com");
    let client = ForgeClient::default();
    let forge_type = client.detect_forge_type(domain);

    tracing::debug!("Detected forge type for {}: {:?}", domain, forge_type);

    let tags = match forge_type {
        ForgeType::GitHub => client.query_github_tags(repo, owner)?,
        ForgeType::Gitea => client.query_gitea_tags(repo, owner, domain)?,
        ForgeType::Unknown => {
            tracing::warn!("Unknown forge type for {}, trying Gitea API", domain);
            client.query_gitea_tags(repo, owner, domain)?
        }
    };

    Ok(tags.into())
}

pub fn get_branches(repo: &str, owner: &str, domain: Option<&str>) -> Result<Branches, ApiError> {
    let domain = domain.unwrap_or("github.com");
    let client = ForgeClient::default();
    let forge_type = client.detect_forge_type(domain);

    tracing::debug!(
        "Fetching branches for {}/{} on {} ({:?})",
        owner,
        repo,
        domain,
        forge_type
    );

    let branches = match forge_type {
        ForgeType::GitHub => client.query_github_branches(repo, owner)?,
        ForgeType::Gitea => client.query_gitea_branches(repo, owner, domain)?,
        ForgeType::Unknown => {
            tracing::warn!("Unknown forge type for {}, trying Gitea API", domain);
            client.query_gitea_branches(repo, owner, domain)?
        }
    };

    Ok(branches.into())
}

/// Check if a specific branch exists without listing all branches.
/// Much more efficient for repos with many branches (like nixpkgs).
pub fn branch_exists(repo: &str, owner: &str, branch: &str, domain: Option<&str>) -> bool {
    let domain = domain.unwrap_or("github.com");
    let client = ForgeClient::default();
    let forge_type = client.detect_forge_type(domain);

    match forge_type {
        ForgeType::GitHub => client.branch_exists_github(repo, owner, branch),
        ForgeType::Gitea => client.branch_exists_gitea(repo, owner, domain, branch),
        ForgeType::Unknown => client.branch_exists_gitea(repo, owner, domain, branch),
    }
}

/// Check multiple branches and return which ones exist.
/// More efficient than get_branches for known candidate branches.
pub fn filter_existing_branches(
    repo: &str,
    owner: &str,
    candidates: &[String],
    domain: Option<&str>,
) -> Vec<String> {
    candidates
        .iter()
        .filter(|branch| branch_exists(repo, owner, branch, domain))
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
