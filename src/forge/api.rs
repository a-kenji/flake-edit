use std::collections::HashMap;
use std::process::Command;
use std::sync::Mutex;
use std::sync::OnceLock;
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

    /// Could not establish a connection.
    #[error("could not reach {url}")]
    ConnectFailed {
        url: String,
        #[source]
        source: SourceError,
    },

    #[error("{url} not found (HTTP 404)")]
    NotFound { url: String },

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
    /// rarer existing variants.
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

/// Classify a `ureq::Error` from establishing the request into the
/// domain `ApiError`. Hand-written rather than `#[from]` so a new
/// ureq variant must be handled explicitly and cannot silently
/// become `Other`.
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
pub(crate) struct Headers {
    pub(crate) user_agent: Option<String>,
    pub(crate) authorization: Option<String>,
}

impl Headers {
    /// Headers with optional Bearer token authentication for the given domain.
    fn for_domain(domain: &str) -> Self {
        let authorization = get_forge_token(domain).map(|token| {
            tracing::debug!("Found token for {}", domain);
            format!("Bearer {token}")
        });
        Self {
            user_agent: Some("flake-edit".to_string()),
            authorization,
        }
    }
}

/// Outcome of a conditional GET. Distinguishes "the cached body is
/// still authoritative" from "here is a fresh body and its new ETag".
pub(crate) enum ConditionalResponse {
    /// The server returned 304: the caller's cached body for this URL
    /// is still authoritative.
    NotModified,
    /// Fresh response. `etag` is `None` when the server did not send
    /// one, in which case the response is uncacheable.
    Body { body: String, etag: Option<String> },
}

/// HTTP layer: one shared `ureq::Agent` with explicit timeouts. The
/// only direct user is [`super::cache::HttpCache`], which adds
/// persistent ETag revalidation on top.
pub(crate) struct HttpClient {
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

    pub(crate) fn get(&self, url: &str, headers: &Headers) -> Result<String, ApiError> {
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
    pub(crate) fn head_status(&self, url: &str, headers: &Headers) -> Result<bool, ApiError> {
        match self.build(url, headers).call() {
            Ok(_) => Ok(true),
            Err(e) => match classify_ureq(e, url) {
                ApiError::NotFound { .. } => Ok(false),
                other => Err(other),
            },
        }
    }

    /// `ureq` reports a 304 as `Error::StatusCode(304)` because it is
    /// outside the 2xx range. We intercept that error variant
    /// specifically so the cache layer sees a clean
    /// `ConditionalResponse::NotModified` and the body-reading path
    /// is reached only for real 2xx responses.
    pub(crate) fn get_conditional(
        &self,
        url: &str,
        headers: &Headers,
        etag: Option<&str>,
    ) -> Result<ConditionalResponse, ApiError> {
        let mut request = self.build(url, headers);
        if let Some(etag) = etag {
            request = request.header("If-None-Match", etag);
        }
        match request.call() {
            Ok(mut response) => {
                let new_etag = response
                    .headers()
                    .get("etag")
                    .and_then(|v| v.to_str().ok())
                    .map(String::from);
                let body = response
                    .body_mut()
                    .read_to_string()
                    .map_err(|e| classify_body_read(e, url))?;
                Ok(ConditionalResponse::Body {
                    body,
                    etag: new_etag,
                })
            }
            Err(ureq::Error::StatusCode(304)) => Ok(ConditionalResponse::NotModified),
            Err(e) => Err(classify_ureq(e, url)),
        }
    }

    pub(crate) fn post_json(
        &self,
        url: &str,
        headers: &Headers,
        body: &str,
    ) -> Result<String, ApiError> {
        let mut request = self
            .agent
            .post(url)
            .header("Content-Type", "application/json");
        if let Some(ref ua) = headers.user_agent {
            request = request.header("User-Agent", ua);
        }
        if let Some(ref auth) = headers.authorization {
            request = request.header("Authorization", auth);
        }
        let response_body = request
            .send(body)
            .map_err(|e| classify_ureq(e, url))?
            .body_mut()
            .read_to_string()
            .map_err(|e| classify_body_read(e, url))?;
        Ok(response_body)
    }
}

/// Hard cap on paginated listing requests.
const MAX_PAGES: u32 = 20;
/// GitHub's maximum page size for list endpoints.
const PER_PAGE: usize = 100;
/// Gitea pagination size. Smaller because self-hosted instances
/// often cap server-side.
const GITEA_PER_PAGE: usize = 50;

/// Must stay in step with the [`IntermediaryTags`] to [`Tags`]
/// conversion: [`ForgeClient::fetch_github_tags`] uses this
/// predicate to decide that page 1 is trustworthy, then hands the
/// same names to the conversion. If the conversion drops a name
/// that the predicate accepted, the cheap path can return a
/// [`Tags`] whose [`Tags::get_latest_tag`] is `None`.
fn parses_as_semver(name: &str) -> bool {
    let parsed = parse_ref(name, false);
    Version::parse(&parsed.normalized_for_semver).is_ok()
}

/// Drive a paginated listing endpoint, accumulating items in order
/// until exhaustion or `max_pages` bounds a misbehaving endpoint.
///
/// `fetch` receives a 1-based page number and returns the items on
/// that page. Iteration stops when `fetch` yields fewer than
/// `per_page` items (real APIs signal "no more" with a short page),
/// or when `page` reaches `max_pages`, which exists so an endpoint
/// that always returns full pages cannot loop indefinitely.
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

/// Unified entry point for talking to forges (GitHub, Gitea/Forgejo).
///
/// One shared HTTP agent and per-run result caches for tags,
/// branches, and branch-exists probes. Locking is fine-grained per
/// cache: two threads racing on the same missing key may both
/// fetch, at the cost of one duplicated round trip.
pub struct ForgeClient {
    http: super::cache::HttpCache,
    tags_cache: Mutex<HashMap<RepoKey, Tags>>,
    branches_cache: Mutex<HashMap<RepoKey, Branches>>,
    branch_exists_cache: Mutex<HashMap<BranchKey, bool>>,
    /// `false` when no github.com token is available; unauthenticated
    /// runs skip the GraphQL batch because the endpoint rejects them
    /// with HTTP 401, and fall back to anonymous REST.
    github_graphql_enabled: bool,
}

/// One unit of work in a [`ForgeClient::batch_warm_github`] call.
///
/// The variants mirror the per-repo round trips the REST path would
/// otherwise make: `Tags` replaces `list_tags`, `ChannelCandidates`
/// replaces a fan of `branch_exists` probes for a known candidate set.
/// Both prime the same caches the REST path consults.
#[derive(Debug, Clone)]
pub(crate) enum BatchLookup {
    /// Prime the tags cache for `github.com/{owner}/{repo}`.
    Tags { owner: String, repo: String },
    /// Prime `branch_exists` for each `candidate` under
    /// `refs/heads/{prefix}`. Candidates not present in the GraphQL
    /// response cache as `false`; ones returned cache as `true`.
    ChannelCandidates {
        owner: String,
        repo: String,
        /// e.g. `"nixos-"`. Drives the `refPrefix` in the GraphQL query.
        prefix: String,
        /// Full branch names the caller will subsequently probe via
        /// `branch_exists`. Pre-computed by the caller so api.rs stays
        /// independent of the channel-version generator.
        candidates: Vec<String>,
    },
}

type RepoKey = (String, String, String);
type BranchKey = (String, String, String, String);

impl Default for ForgeClient {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ForgeClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForgeClient").finish_non_exhaustive()
    }
}

impl ForgeClient {
    /// Build a client backed by the XDG-located persistent ETag cache.
    pub fn new() -> Self {
        Self {
            http: super::cache::HttpCache::new(),
            tags_cache: Mutex::new(HashMap::new()),
            branches_cache: Mutex::new(HashMap::new()),
            branch_exists_cache: Mutex::new(HashMap::new()),
            github_graphql_enabled: get_forge_token("github.com").is_some(),
        }
    }

    fn canonical_domain(domain: Option<&str>) -> String {
        domain.unwrap_or("github.com").to_string()
    }

    /// Latest-known tags for `(owner, repo)` at `domain`. Cached on success.
    pub fn list_tags(
        &self,
        owner: &str,
        repo: &str,
        domain: Option<&str>,
    ) -> Result<Tags, ApiError> {
        let key = (
            Self::canonical_domain(domain),
            owner.to_string(),
            repo.to_string(),
        );
        if let Some(hit) = self
            .tags_cache
            .lock()
            .expect("forge tags cache poisoned")
            .get(&key)
            .cloned()
        {
            return Ok(hit);
        }
        let fresh = if key.0 == "github.com" {
            self.fetch_github_tags(owner, repo)?
        } else {
            self.fetch_gitea_tags(&key.0, owner, repo)?
        };
        self.tags_cache
            .lock()
            .expect("forge tags cache poisoned")
            .insert(key, fresh.clone());
        Ok(fresh)
    }

    /// All branch names for `(owner, repo)` at `domain`. Cached on success.
    pub fn list_branches(
        &self,
        owner: &str,
        repo: &str,
        domain: Option<&str>,
    ) -> Result<Branches, ApiError> {
        let key = (
            Self::canonical_domain(domain),
            owner.to_string(),
            repo.to_string(),
        );
        if let Some(hit) = self
            .branches_cache
            .lock()
            .expect("forge branches cache poisoned")
            .get(&key)
            .cloned()
        {
            return Ok(hit);
        }
        let fresh = if key.0 == "github.com" {
            self.fetch_github_branches(owner, repo)?
        } else {
            self.fetch_gitea_branches(&key.0, owner, repo)?
        };
        self.branches_cache
            .lock()
            .expect("forge branches cache poisoned")
            .insert(key, fresh.clone());
        Ok(fresh)
    }

    /// Single-branch existence probe.
    ///
    /// `Ok(true)` for an existing branch, `Ok(false)` for a forge that
    /// returned 404, `Err(_)` for any transient failure (timeout,
    /// DNS, 5xx, ...). Both `Ok(true)` and `Ok(false)` are cached;
    /// errors are not.
    pub fn branch_exists(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
        domain: Option<&str>,
    ) -> Result<bool, ApiError> {
        let key = (
            Self::canonical_domain(domain),
            owner.to_string(),
            repo.to_string(),
            branch.to_string(),
        );
        if let Some(&hit) = self
            .branch_exists_cache
            .lock()
            .expect("forge branch_exists cache poisoned")
            .get(&key)
        {
            return Ok(hit);
        }
        let fresh = if key.0 == "github.com" {
            self.fetch_github_branch_exists(owner, repo, branch)?
        } else {
            self.fetch_gitea_branch_exists(&key.0, owner, repo, branch)?
        };
        self.branch_exists_cache
            .lock()
            .expect("forge branch_exists cache poisoned")
            .insert(key, fresh);
        Ok(fresh)
    }

    /// Resolve many `github.com` lookups in one GraphQL POST and
    /// prime the per-run caches with the results.
    ///
    /// `Tags` lookups populate `tags_cache`; `ChannelCandidates`
    /// lookups populate `branch_exists_cache` for every candidate
    /// (returned branches as `true`, missing ones as `false`).
    /// Subsequent calls to [`ForgeClient::list_tags`] and
    /// [`ForgeClient::branch_exists`] for the same `(owner, repo)`
    /// then hit the cache instead of issuing per-repo REST round
    /// trips.
    ///
    /// Per-input errors do not abort the batch. A repo that GitHub
    /// returns as `null` (private, missing) leaves its cache slot
    /// empty so the caller falls through to the REST path, which
    /// surfaces the underlying error in context. The whole call
    /// returns `Err` only when the POST itself fails.
    pub(crate) fn batch_warm_github(&self, lookups: &[BatchLookup]) -> Result<usize, ApiError> {
        if !self.github_graphql_enabled || lookups.is_empty() {
            return Ok(0);
        }
        let headers = Headers::for_domain("github.com");
        let (query, aliases) = build_graphql_query(lookups);
        let payload = serde_json::json!({ "query": query }).to_string();
        let url = "https://api.github.com/graphql";
        tracing::debug!(
            "Batching {} github.com lookup(s) into one GraphQL POST",
            aliases.len()
        );
        let body = self.http.post_json(url, &headers, &payload)?;
        let parsed: GraphQlResponse =
            serde_json::from_str(&body).map_err(|source| ApiError::Json {
                url: url.to_string(),
                source,
            })?;

        let mut primed = 0usize;
        for (alias, lookup) in &aliases {
            let Some(node) = parsed.data.as_ref().and_then(|d| d.get(alias)) else {
                continue;
            };
            let Some(repo) = node.as_ref() else {
                // GraphQL returned null for this repo (private, missing,
                // or partial-errors). Skip and let REST surface the
                // error in context.
                tracing::debug!("GraphQL returned null for alias {}", alias);
                continue;
            };
            let names: Vec<String> = repo
                .refs
                .as_ref()
                .map(|r| r.nodes.iter().map(|n| n.name.clone()).collect())
                .unwrap_or_default();
            match lookup {
                BatchLookup::Tags { owner, repo: r } => {
                    let inter = IntermediaryTags(
                        names
                            .into_iter()
                            .map(|name| IntermediaryTag { name })
                            .collect(),
                    );
                    let tags: Tags = inter.into();
                    let key = ("github.com".to_string(), owner.clone(), r.clone());
                    self.tags_cache
                        .lock()
                        .expect("forge tags cache poisoned")
                        .insert(key, tags);
                    primed += 1;
                }
                BatchLookup::ChannelCandidates {
                    owner,
                    repo: r,
                    candidates,
                    ..
                } => {
                    let returned: std::collections::HashSet<&str> =
                        names.iter().map(|n| n.as_str()).collect();
                    let mut cache = self
                        .branch_exists_cache
                        .lock()
                        .expect("forge branch_exists cache poisoned");
                    for candidate in candidates {
                        let key = (
                            "github.com".to_string(),
                            owner.clone(),
                            r.clone(),
                            candidate.clone(),
                        );
                        cache.insert(key, returned.contains(candidate.as_str()));
                    }
                    primed += 1;
                }
            }
        }
        Ok(primed)
    }

    /// Fetch tags from `github.com/{owner}/{repo}`, trusting page 1
    /// when it contains at least one parseable semver tag.
    ///
    /// GitHub's `/tags` orders by ref creation time, not by semver,
    /// so the cheap path is right for monotone version progressions
    /// and falls back to a paginated walk capped at [`MAX_PAGES`]
    /// when page 1 is full but contains no semver-parseable names
    /// (hash- or date-style tagging schemes). Repos that backport
    /// onto an older branch can push the latest major off page 1;
    /// users hitting that must pin manually.
    fn fetch_github_tags(&self, owner: &str, repo: &str) -> Result<Tags, ApiError> {
        let headers = Headers::for_domain("github.com");
        let url = |page: u32| {
            format!(
                "https://api.github.com/repos/{owner}/{repo}/tags?per_page={PER_PAGE}&page={page}"
            )
        };

        let first_url = url(1);
        tracing::debug!("Fetching tags page 1: {}", first_url);
        let body = self.http.get(&first_url, &headers)?;
        let first: IntermediaryTags =
            serde_json::from_str(&body).map_err(|source| ApiError::Json {
                url: first_url.clone(),
                source,
            })?;
        let mut all = first.0;
        let first_was_full = all.len() >= PER_PAGE;
        let first_has_semver = all.iter().any(|t| parses_as_semver(&t.name));

        if !first_was_full || first_has_semver {
            tracing::debug!(
                "Cheap path returned {} tag(s) from page 1 (full={}, has_semver={})",
                all.len(),
                first_was_full,
                first_has_semver
            );
            return Ok(IntermediaryTags(all).into());
        }

        tracing::debug!(
            "Page 1 had no parseable semver in a full page; falling back to paginated walk"
        );
        for page in 2..=MAX_PAGES {
            let page_url = url(page);
            tracing::debug!("Fetching tags page {}: {}", page, page_url);
            let body = self.http.get(&page_url, &headers)?;
            let next: IntermediaryTags =
                serde_json::from_str(&body).map_err(|source| ApiError::Json {
                    url: page_url.clone(),
                    source,
                })?;
            let count = next.0.len();
            all.extend(next.0);
            if count < PER_PAGE {
                break;
            }
        }
        tracing::debug!("Total tags fetched: {}", all.len());
        Ok(IntermediaryTags(all).into())
    }

    fn fetch_github_branches(&self, owner: &str, repo: &str) -> Result<Branches, ApiError> {
        let headers = Headers::for_domain("github.com");
        let branches = paginated(PER_PAGE, MAX_PAGES, |page| {
            let url = format!(
                "https://api.github.com/repos/{owner}/{repo}/branches?per_page={PER_PAGE}&page={page}"
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

    fn fetch_github_branch_exists(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> Result<bool, ApiError> {
        let headers = Headers::for_domain("github.com");
        let url = format!("https://api.github.com/repos/{owner}/{repo}/branches/{branch}");
        self.http.head_status(&url, &headers)
    }

    fn fetch_gitea_tags(&self, domain: &str, owner: &str, repo: &str) -> Result<Tags, ApiError> {
        let headers = Headers::for_domain(domain);

        // Try HTTPS, fall back to HTTP.
        for scheme in ["https", "http"] {
            let url = format!("{scheme}://{domain}/api/v1/repos/{owner}/{repo}/tags");
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

    fn fetch_gitea_branches(
        &self,
        domain: &str,
        owner: &str,
        repo: &str,
    ) -> Result<Branches, ApiError> {
        let headers = Headers::for_domain(domain);
        let mut all_branches = Vec::new();
        let mut page = 1;

        for scheme in ["https", "http"] {
            loop {
                let url = format!(
                    "{scheme}://{domain}/api/v1/repos/{owner}/{repo}/branches?limit={GITEA_PER_PAGE}&page={page}"
                );
                tracing::debug!("Trying Gitea branches endpoint: {}", url);

                match self.http.get(&url, &headers) {
                    Ok(body) => {
                        tracing::debug!("Body from Gitea API: {body}");
                        match serde_json::from_str::<IntermediaryBranches>(&body) {
                            Ok(page_branches) => {
                                let count = page_branches.0.len();
                                all_branches.extend(page_branches.0);

                                if count < GITEA_PER_PAGE || page >= MAX_PAGES {
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

    fn fetch_gitea_branch_exists(
        &self,
        domain: &str,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> Result<bool, ApiError> {
        let headers = Headers::for_domain(domain);
        // Try https, fall back to http. The first scheme to give a
        // definitive answer (2xx or 404) wins; http is tried only
        // when https errored out before reaching the application
        // layer.
        let mut last_err: Option<ApiError> = None;
        for scheme in ["https", "http"] {
            let url = format!("{scheme}://{domain}/api/v1/repos/{owner}/{repo}/branches/{branch}");
            match self.http.head_status(&url, &headers) {
                Ok(answer) => return Ok(answer),
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.expect("at least one scheme was attempted"))
    }
}

/// Build a single GraphQL document that resolves every `lookup`.
///
/// Each lookup is wrapped in an aliased `repository(owner:, name:)`
/// query (`r0`, `r1`, ...) so the response can be demultiplexed by
/// alias even when two lookups target the same repo. GitHub GraphQL
/// requires unique aliases when the same field appears more than
/// once in a selection set; the index suffix guarantees uniqueness
/// regardless of how the caller deduplicates upstream.
fn build_graphql_query(lookups: &[BatchLookup]) -> (String, Vec<(String, BatchLookup)>) {
    let mut query = String::from("query {\n");
    let mut aliases = Vec::with_capacity(lookups.len());
    for (i, lookup) in lookups.iter().enumerate() {
        let alias = format!("r{i}");
        let (owner, repo) = match lookup {
            BatchLookup::Tags { owner, repo } => (owner, repo),
            BatchLookup::ChannelCandidates { owner, repo, .. } => (owner, repo),
        };
        query.push_str(&format!(
            "  {alias}: repository(owner:{owner}, name:{repo}) {{\n",
            owner = json_string(owner),
            repo = json_string(repo),
        ));
        match lookup {
            BatchLookup::Tags { .. } => {
                // `orderBy: TAG_COMMIT_DATE DESC` is what makes the
                // first 100 refs comparable to REST's
                // `/tags?per_page=100&page=1`. Without it, GitHub
                // returns refs in lexicographic order, which for a
                // repo with more than 100 tags can push the
                // highest-semver tag off the window and silently
                // cache a stale "latest". The REST cheap path
                // defends with a paginated fallback; the GraphQL
                // path has no equivalent, so the ordering hint is
                // load-bearing rather than cosmetic.
                query.push_str(
                    "    refs(refPrefix:\"refs/tags/\", first:100, \
                     orderBy:{field: TAG_COMMIT_DATE, direction: DESC}) {\n",
                );
            }
            BatchLookup::ChannelCandidates { prefix, .. } => {
                // `first:100` is GitHub GraphQL's hard cap for `refs`.
                // For the channel prefixes we care about (`nixos-`,
                // `nixpkgs-`, `release-`, `nix-darwin-`) the universe
                // of branches is far smaller, but the cap leaves
                // headroom for forks that accumulate stale release
                // branches without truncating real candidates into
                // false-negatives.
                query.push_str(&format!(
                    "    refs(refPrefix:{p}, first:100) {{\n",
                    p = json_string(&format!("refs/heads/{prefix}")),
                ));
            }
        }
        query.push_str("      nodes { name }\n");
        query.push_str("    }\n");
        query.push_str("  }\n");
        aliases.push((alias, lookup.clone()));
    }
    query.push_str("}\n");
    (query, aliases)
}

/// Escape `s` into a JSON-quoted string suitable for inlining as a
/// GraphQL argument literal. `serde_json::to_string` is reused
/// instead of hand-rolling escapes so backslash, quote, and control
/// character handling stays correct.
fn json_string(s: &str) -> String {
    serde_json::Value::String(s.to_string()).to_string()
}

/// GraphQL response shape. Both lookup kinds collapse to the same
/// `repository { refs { nodes { name } } }` projection on the wire,
/// so one struct covers both.
#[derive(Deserialize, Debug)]
struct GraphQlResponse {
    /// Absent when GitHub returned only an `errors` block (e.g. 401);
    /// each entry can be `None` when a single repo failed inside an
    /// otherwise-successful response.
    data: Option<HashMap<String, Option<GraphQlRepo>>>,
}

#[derive(Deserialize, Debug)]
struct GraphQlRepo {
    refs: Option<GraphQlRefs>,
}

#[derive(Deserialize, Debug)]
struct GraphQlRefs {
    nodes: Vec<GraphQlRefName>,
}

#[derive(Deserialize, Debug)]
struct GraphQlRefName {
    name: String,
}

#[derive(Deserialize, Debug)]
struct IntermediaryTags(Vec<IntermediaryTag>);

#[derive(Deserialize, Debug)]
struct IntermediaryBranches(Vec<IntermediaryBranch>);

#[derive(Deserialize, Debug)]
struct IntermediaryBranch {
    name: String,
}

#[derive(Debug, Default, Clone)]
pub struct Branches {
    pub names: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Tags {
    versions: Vec<TagVersion>,
}

impl Tags {
    /// Latest semver-ordered tag, or `None` for an empty / fully
    /// unparseable set.
    pub fn get_latest_tag(&self) -> Option<String> {
        self.versions
            .iter()
            .max_by(|a, b| a.version.cmp_precedence(&b.version))
            .map(|tag| tag.original.clone())
    }
}

#[derive(Deserialize, Debug)]
struct IntermediaryTag {
    name: String,
}

#[derive(Debug, Clone)]
struct TagVersion {
    version: Version,
    original: String,
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

/// Per-process cache of resolved forge tokens, keyed by domain.
///
/// Scope is per-process because the resolver inputs (`nix.conf`,
/// `GITHUB_TOKEN` and friends) do not change during a `flake-edit`
/// invocation; we never re-read them. `None` is cached the same
/// as `Some(_)` so a domain with no configured token does not
/// re-fork `nix config show --json` on every request.
fn token_cache() -> &'static Mutex<HashMap<String, Option<String>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn get_forge_token(domain: &str) -> Option<String> {
    {
        let cache = token_cache().lock().expect("forge token cache poisoned");
        if let Some(cached) = cache.get(domain) {
            return cached.clone();
        }
    }
    // Resolve outside the lock so a slow `nix` fork does not
    // block lookups for unrelated domains. A racing duplicate
    // resolution is harmless: `or_insert_with` keeps whichever
    // value was inserted first, and both racers compute the same
    // answer for the same domain.
    let resolved = resolve_forge_token(domain);
    let mut cache = token_cache().lock().expect("forge token cache poisoned");
    cache
        .entry(domain.to_string())
        .or_insert_with(|| resolved.clone())
        .clone()
}

/// Resolve `domain`'s forge token from scratch, with no caching.
///
/// Forks `nix config show --json` on every call. Callers must
/// route through [`get_forge_token`] so repeat lookups do not
/// re-fork.
fn resolve_forge_token(domain: &str) -> Option<String> {
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

    // Last-resort fallback: shell out to `gh auth token --hostname
    // <domain>`. A user who only ran `gh auth login` is anonymous to
    // every check above (the gh CLI stores its token in its own
    // credential file, not in nix.conf or the environment), so this
    // single fork is what lifts that population from the 60/hr
    // anonymous rate limit onto a real token. `gh` exits non-zero
    // when it has no token for the host, which we treat as "no
    // token" and fall through.
    if let Ok(output) = Command::new("gh")
        .args(["auth", "token", "--hostname", domain])
        .output()
        && output.status.success()
        && let Ok(stdout) = String::from_utf8(output.stdout)
    {
        let token = stdout.trim();
        if !token.is_empty() {
            return Some(token.to_string());
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
        let result = paginated::<u32, _>(100, MAX_PAGES, |page| {
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
        let tags: Tags = intermediary.into();

        assert_eq!(tags.get_latest_tag(), Some("refs/tags/v2.0.0".to_string()));
    }

    #[test]
    fn tags_parsing_with_short_versions() {
        let json = r#"[
            {"name": "v1"},
            {"name": "v1.1"}
        ]"#;

        let intermediary: IntermediaryTags = serde_json::from_str(json).unwrap();
        let tags: Tags = intermediary.into();

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
        let tags: Tags = intermediary.into();

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
        let tags: Tags = intermediary.into();

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
        let tags: Tags = intermediary.into();

        assert_eq!(tags.get_latest_tag(), Some("v2.0.0".to_string()));
    }

    #[test]
    fn tags_parsing_empty() {
        let json = r#"[]"#;

        let intermediary: IntermediaryTags = serde_json::from_str(json).unwrap();
        let tags: Tags = intermediary.into();

        assert_eq!(tags.get_latest_tag(), None);
    }

    #[test]
    fn tags_parsing_orders_prereleases_by_semver_precedence() {
        let json = r#"[
            {"name": "v1.0.0"},
            {"name": "v2.0.0-beta.1"},
            {"name": "v1.5.0"}
        ]"#;

        let intermediary: IntermediaryTags = serde_json::from_str(json).unwrap();
        let tags: Tags = intermediary.into();

        // `v2.0.0-beta.1` beats `v1.5.0` because semver precedence
        // compares major before flagging prerelease status; a higher
        // major wins even when the higher tag is a prerelease.
        assert_eq!(tags.get_latest_tag(), Some("v2.0.0-beta.1".to_string()));
    }

    #[test]
    fn tags_parsing_handles_hl_prefixed_scheme_without_downgrade() {
        let json = r#"[
            {"name": "hl0.21.0-1"},
            {"name": "hl0.33.0-1"},
            {"name": "hl0.46.0-1"},
            {"name": "hl0.47.0-1"}
        ]"#;

        let intermediary: IntermediaryTags = serde_json::from_str(json).unwrap();
        let tags: Tags = intermediary.into();

        assert_eq!(tags.get_latest_tag(), Some("hl0.47.0-1".to_string()));
    }

    #[test]
    fn tags_parsing_combined_prefixes() {
        let json = r#"[
            {"name": "refs/tags/v1.0.0"},
            {"name": "refs/tags/v2.0.0"}
        ]"#;

        let intermediary: IntermediaryTags = serde_json::from_str(json).unwrap();
        let tags: Tags = intermediary.into();

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
        let tags: Tags = intermediary.into();

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

    #[test]
    fn forge_client_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ForgeClient>();
    }

    #[test]
    fn parses_as_semver_recognizes_normalized_shapes() {
        // Pins the agreement between the cheap-path early-stop and
        // the `IntermediaryTags -> Tags` conversion. See the doc on
        // `parses_as_semver` for why divergence is unsound.
        assert!(parses_as_semver("v1.2.3"));
        assert!(parses_as_semver("refs/tags/v1.2.3"));
        assert!(parses_as_semver("release-1.2.3"));
        assert!(parses_as_semver("v1")); // normalized to v1.0.0
        assert!(parses_as_semver("1.0.0+gitea")); // build metadata is valid semver
        assert!(parses_as_semver("release-1.5")); // 2-segment pads to 1.5.0
        assert!(!parses_as_semver("invalid-tag"));
        assert!(!parses_as_semver("abc"));
        assert!(!parses_as_semver(""));
        // Leading-zero rejection: `release-25.05` normalizes to
        // `25.05.0`, which the semver crate rejects because `05` has
        // a leading zero. Channel-style year.month refs must NOT be
        // accepted by the cheap path: a flake using `release-25.05`
        // (or `release-24.05`) on a github.com repo would otherwise
        // be mis-classified as a semver tag and short-circuit the
        // tag walk against a window that does not contain it.
        assert!(!parses_as_semver("release-25.05"));
        assert!(!parses_as_semver("release-24.05"));
        // Prereleases must satisfy the predicate so that a page-1
        // listing of prerelease tags still trips the cheap-path
        // early-stop. Selection downstream picks a stable release
        // on the same page when one exists.
        assert!(parses_as_semver("v1.2.3-rc1"));
    }

    #[test]
    fn build_graphql_query_uses_distinct_aliases() {
        // GitHub GraphQL requires each `repository(...)` selection in
        // one document to use a unique alias. Two lookups that target
        // the same `(owner, repo)` must still get distinct aliases,
        // otherwise the POST is rejected before any response can be
        // demultiplexed.
        let lookups = vec![
            BatchLookup::Tags {
                owner: "same".into(),
                repo: "same".into(),
            },
            BatchLookup::Tags {
                owner: "same".into(),
                repo: "same".into(),
            },
        ];
        let (query, aliases) = build_graphql_query(&lookups);
        assert_eq!(aliases.len(), 2);
        assert_eq!(aliases[0].0, "r0");
        assert_eq!(aliases[1].0, "r1");
        assert!(
            query.contains("r0:"),
            "first lookup must use the r0 alias; query was:\n{query}"
        );
        assert!(
            query.contains("r1:"),
            "second lookup must use a distinct r1 alias; query was:\n{query}"
        );
    }
}
