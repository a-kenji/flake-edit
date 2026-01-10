use std::collections::HashMap;
use std::process::Command;

use reqwest::blocking::Client;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue, USER_AGENT};
use semver::Version;
use serde::Deserialize;
use thiserror::Error;

use crate::version::parse_ref;
#[derive(Error, Debug)]
pub enum ApiError {
    #[error("HTTP request failed: {0}")]
    HttpError(#[from] reqwest::Error),

    #[error("JSON parsing failed: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Invalid header value: {0}")]
    InvalidHeader(#[from] reqwest::header::InvalidHeaderValue),

    #[error("UTF-8 conversion failed: {0}")]
    Utf8Error(#[from] std::string::FromUtf8Error),

    #[error("Failed to execute command: {0}")]
    CommandError(#[from] std::io::Error),

    #[error("No tags found for repository")]
    NoTagsFound,

    #[error("Invalid domain or repository: {0}")]
    InvalidInput(String),
}

#[derive(Default)]
pub struct ForgeClient {
    client: Client,
}

impl ForgeClient {
    fn get(&self, url: &str, headers: HeaderMap) -> Result<String, ApiError> {
        let response = self.client.get(url).headers(headers).send()?;
        let text = response.text()?;
        Ok(text)
    }

    fn make_headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Ok(user_agent) = HeaderValue::from_str("flake-edit") {
            headers.insert(USER_AGENT, user_agent);
        }
        headers
    }

    pub fn detect_forge_type(&self, domain: &str) -> ForgeType {
        if domain == "github.com" {
            return ForgeType::GitHub;
        }

        tracing::debug!("Attempting to detect forge type for domain: {}", domain);
        let headers = Self::make_headers();

        // Try both HTTPS and HTTP for each endpoint
        for scheme in ["https", "http"] {
            // Try Forgejo version endpoint first
            let forgejo_url = format!("{}://{}/api/forgejo/v1/version", scheme, domain);
            tracing::debug!("Trying Forgejo endpoint: {}", forgejo_url);
            if let Ok(text) = self.get(&forgejo_url, headers.clone()) {
                tracing::debug!("Forgejo endpoint response body: {}", text);
                if let Some(forge_type) = parse_forge_version(&text) {
                    tracing::info!("Detected Forgejo/Gitea at {}", domain);
                    return forge_type;
                }
            }

            // Try Gitea version endpoint
            let gitea_url = format!("{}://{}/api/v1/version", scheme, domain);
            tracing::debug!("Trying Gitea endpoint: {}", gitea_url);
            if let Ok(text) = self.get(&gitea_url, headers.clone()) {
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
        let mut headers = Self::make_headers();
        if let Some(token) = get_forge_token("github.com") {
            tracing::debug!("Found github token.");
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {token}"))?,
            );
        }

        let body = self.get(
            &format!("https://api.github.com/repos/{}/{}/tags", owner, repo),
            headers,
        )?;

        tracing::debug!("Body from api: {body}");
        let tags = serde_json::from_str::<IntermediaryTags>(&body)?;
        Ok(tags)
    }

    pub fn query_gitea_tags(
        &self,
        repo: &str,
        owner: &str,
        domain: &str,
    ) -> Result<IntermediaryTags, ApiError> {
        let mut headers = Self::make_headers();

        if let Some(token) = get_forge_token(domain) {
            tracing::debug!("Found token for {}", domain);
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {token}"))?,
            );
        }

        // Try HTTPS first, then HTTP
        for scheme in ["https", "http"] {
            let url = format!(
                "{}://{}/api/v1/repos/{}/{}/tags",
                scheme, domain, owner, repo
            );
            tracing::debug!("Trying Gitea tags endpoint: {}", url);

            if let Ok(body) = self.get(&url, headers.clone()) {
                tracing::debug!("Body from Gitea API: {body}");
                if let Ok(tags) = serde_json::from_str::<IntermediaryTags>(&body) {
                    return Ok(tags);
                }
            }
        }

        Err(ApiError::NoTagsFound)
    }
}

#[derive(Deserialize, Debug)]
pub struct IntermediaryTags(Vec<IntermediaryTag>);

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
    {
        if let Ok(stdout) = String::from_utf8(output.stdout) {
            if let Ok(config) = serde_json::from_str::<NixConfig>(&stdout) {
                if let Some(token) = config.forge_token(domain) {
                    return Some(token);
                }
            }
        }
    }

    // Fallback to environment variables
    if let Ok(token) = std::env::var("GITEA_TOKEN") {
        return Some(token);
    }
    if let Ok(token) = std::env::var("FORGEJO_TOKEN") {
        return Some(token);
    }
    if domain == "github.com" {
        if let Ok(token) = std::env::var("GITHUB_TOKEN") {
            return Some(token);
        }
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
