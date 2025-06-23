use std::collections::HashMap;
use std::process::Command;

use reqwest::blocking::Client;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue, USER_AGENT};
use semver::Version;
use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct IntermediaryTags(Vec<IntermediaryTag>);

#[derive(Debug)]
pub struct Tags {
    versions: Vec<Version>,
    prefix: String,
}

impl Tags {
    pub fn get_latest_tag(&mut self) -> Option<String> {
        self.sort();
        let mut buf = String::new();
        buf.push_str(&self.prefix);
        let latest_version = &self.versions.iter().last()?;
        buf.push_str(&latest_version.to_string());
        Some(buf)
    }
    pub fn sort(&mut self) {
        self.versions.sort_by(Version::cmp_precedence);
    }
}

#[derive(Deserialize, Debug)]
pub struct IntermediaryTag {
    name: String,
}

// TODO: actual error handling
pub fn get_tags(repo: &str, owner: &str) -> Result<Tags, ()> {
    let tags = query_tags(repo, owner).unwrap();
    Ok(tags.into())
}

#[derive(Deserialize, Debug, Clone)]
struct NixConfig {
    #[serde(rename = "access-tokens")]
    access_tokens: Option<AccessTokens>,
}

impl NixConfig {
    fn gh_token(&self) -> Option<String> {
        self.access_tokens
            .clone()
            .unwrap()
            .value
            .get("github.com")
            .cloned()
    }
}

#[derive(Deserialize, Debug, Clone)]
struct AccessTokens {
    value: HashMap<String, String>,
}

// Try to query gh access tokens
pub fn get_gh_token() -> Option<String> {
    let command = Command::new("nix")
        .arg("config")
        .arg("show")
        .arg("--json")
        .output()
        .unwrap();
    let stdout = String::from_utf8(command.stdout).unwrap();
    let output: NixConfig = serde_json::from_str(&stdout).unwrap();

    if let Some(token) = output.gh_token() {
        return Some(token);
    };
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        return Some(token);
    };

    None
}

// https://api.github.com/repos/{OWNER}/{REPO}/tags
// Query tags for github currently.
// TODO: support other forges.
fn query_tags(repo: &str, owner: &str) -> Result<IntermediaryTags, ()> {
    let client = Client::new();
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_str("flake-edit").unwrap());
    if let Some(token) = get_gh_token() {
        tracing::debug!("Found github token.");
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );
        tracing::debug!("Settings github token.");
    }
    let body = client
        .get(format!(
            "https://api.github.com/repos/{}/{}/tags",
            repo, owner
        ))
        .headers(headers)
        .send()
        .unwrap()
        .text()
        .unwrap();

    tracing::debug!("Body from api: {body}");

    match serde_json::from_str::<IntermediaryTags>(&body) {
        Ok(tags) => Ok(tags),
        Err(e) => {
            tracing::error!("Error from api: {e}");
            Err(())
        }
    }
}

impl From<IntermediaryTags> for Tags {
    fn from(value: IntermediaryTags) -> Self {
        fn strip_until_char(s: &str, c: char) -> Option<(String, String)> {
            s.find(c).map(|index| {
                let prefix = s[..index].to_string();
                let remaining = s[index + 1..].to_string();
                (prefix, remaining)
            })
        }
        let mut versions = vec![];
        let mut prefix = String::new();
        for itag in value.0 {
            let mut tag = itag.name;
            // TODO: implement a generic way to find the version prefixes
            if let Some(new_tag) = tag.strip_prefix('v') {
                tag = new_tag.to_string();
                prefix = "v".to_string();
            }

            if let Some((new_prefix, new_tag)) = strip_until_char(&tag, '-') {
                tag = new_tag;
                prefix = format!("{new_prefix}-").to_string();
            }

            match Version::parse(&tag) {
                Ok(semver) => {
                    versions.push(semver);
                }
                Err(e) => {
                    tracing::error!("Could not parse version {:?}", e);
                }
            }
        }
        Tags { versions, prefix }
    }
}
