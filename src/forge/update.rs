use nix_uri::{FlakeRef, RefKind};
use ropey::Rope;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::sync::Mutex;

use super::api::{BatchLookup, ForgeClient};
use super::archive::ArchiveUrl;
use super::channel::{
    ChannelType, UpdateStrategy, channel_probe_candidates, detect_strategy, find_latest_channel,
    parse_channel_ref,
};
use super::version::{is_downgrade, parse_ref};
use crate::edit::InputMap;
use crate::input::Input;
use crate::uri::is_git_url;

/// Cap on concurrent forge requests during the fetch phase. Overlaps
/// per-request latency without tripping anonymous rate limits.
const FETCH_CONCURRENCY: usize = 4;

/// Rewrites flake.nix URIs for `update`, `pin`, and `unpin`.
///
/// Owns one [`ForgeClient`] for the lifetime of the updater so the
/// inner forge fetches share a single HTTP agent and memoize repeats
/// across all inputs in one pass.
#[derive(Debug)]
pub struct Updater {
    text: Rope,
    inputs: Vec<UpdateInput>,
    /// Cumulative char delta from edits applied earlier in the current pass.
    /// Measured in *characters*, since ropey indexes by char.
    offset: i32,
    client: ForgeClient,
}

/// Per-input outcome from the fetch phase.
///
/// Kept separate from the edit phase so multiple inputs can race on
/// the forge while edits to the source text stay strictly sequential
/// and in source order.
struct UpdatePlan {
    /// `ref_or_rev` as it stood before this update.
    ///
    /// Empty when `init` is on and the input was unpinned: the edit
    /// phase prints `Initialized ...` in that case.
    previous_ref: String,
    /// Final ref string after normalisation (`refs/tags/` /
    /// `refs/heads/` re-applied where needed). Compared against
    /// `previous_ref` to decide whether to emit `already on the
    /// latest version`.
    final_change: String,
    updated_uri: String,
}

impl Updater {
    fn print_update_status(id: &str, previous_version: &str, final_change: &str) -> bool {
        let is_up_to_date = previous_version == final_change;
        let initialized = previous_version.is_empty();

        if is_up_to_date {
            println!(
                "{} is already on the latest version: {previous_version}.",
                id
            );
            return false;
        }

        if initialized {
            println!("Initialized {} version pin at {final_change}.", id);
        } else {
            println!("Updated {} from {previous_version} to {final_change}.", id);
        }

        true
    }

    /// Build an [`Updater`] from `text` and the inputs in `map`. Inputs
    /// without an editable URL are skipped.
    pub fn new(text: Rope, map: InputMap) -> Self {
        let client = ForgeClient::new();
        let mut inputs = vec![];
        for (_id, input) in map {
            if !input.has_editable_url() {
                continue;
            }
            // `Input::range` carries rnix `TextRange` *byte* offsets, but ropey
            // indexes by *character*. Convert once against the pristine text so
            // later in-place edits can use simple char-offset arithmetic.
            let url_start = text.byte_to_char(input.range.start) + 1;
            let url_end = text.byte_to_char(input.range.end) - 1;
            inputs.push(UpdateInput {
                input,
                url_start,
                url_end,
            });
        }
        Self {
            inputs,
            text,
            offset: 0,
            client,
        }
    }

    /// Char-index range of the URL string *contents* (without the surrounding `"`),
    /// adjusted for earlier in-place edits.
    fn url_char_range(&self, input: &UpdateInput) -> (usize, usize) {
        let start = (input.url_start as i32 + self.offset) as usize;
        let end = (input.url_end as i32 + self.offset) as usize;
        (start, end)
    }
    fn get_index(&self, id: &str) -> Option<usize> {
        let bare = id
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or(id);
        self.inputs
            .iter()
            .position(|n| n.input.id().as_str() == bare)
    }
    /// Pin the input named `id` to `rev`.
    ///
    /// # Errors
    ///
    /// Returns the requested `id` if no such input exists.
    pub fn pin_input_to_ref(&mut self, id: &str, rev: &str) -> Result<(), String> {
        self.sort();
        let idx = self.get_index(id).ok_or_else(|| id.to_string())?;
        let input = self.inputs[idx].clone();
        tracing::debug!("Input: {:?}", input);
        self.change_input_to_rev(&input, rev);
        Ok(())
    }

    /// Remove any `?ref=` or `?rev=` pin from `id`.
    ///
    /// # Errors
    ///
    /// Returns the requested `id` if no such input exists.
    pub fn unpin_input(&mut self, id: &str) -> Result<(), String> {
        self.sort();
        let idx = self.get_index(id).ok_or_else(|| id.to_string())?;
        let input = self.inputs[idx].clone();
        tracing::debug!("Input: {:?}", input);
        self.remove_ref_and_rev(&input);
        Ok(())
    }

    pub fn update_all_to_latest_semver(&mut self, init: bool) {
        self.update_matching(|_| true, init);
    }

    /// Update only the inputs whose id appears in `ids`.
    ///
    /// IDs that do not name an editable input are silently skipped, the
    /// same way the all-inputs path skips inputs without an editable URL.
    /// Duplicates collapse: each matching input is processed at most once.
    pub fn update_inputs_to_latest_semver(&mut self, ids: &[&str], init: bool) {
        if ids.is_empty() {
            return;
        }
        let set: HashSet<&str> = ids.iter().copied().collect();
        self.update_matching(|id| set.contains(id), init);
    }

    /// Two-phase update over the inputs whose id satisfies `keep`.
    ///
    /// Phase 1 fans the fetch out across a bounded worker pool so the
    /// per-input forge round-trips overlap. Phase 2 walks the results
    /// in source order and applies each rewrite serially, because
    /// [`Updater::update_input`]'s cumulative `offset` arithmetic is
    /// only valid when edits land left-to-right in the source text.
    ///
    /// Status prints (`Updated X ...`, `Initialized X ...`, `X is
    /// already on the latest version`) are emitted by the edit phase
    /// so they too appear in source order, regardless of the order
    /// in which workers actually finished their fetches.
    fn update_matching<F: Fn(&str) -> bool>(&mut self, keep: F, init: bool) {
        self.sort();

        // Snapshot URIs against the pristine source text. `self.offset`
        // is zero on entry, so [`Self::get_input_text`] returns exactly
        // what's in the original source; later edit-phase rewrites
        // shift the offset and would otherwise corrupt these slices.
        let pending: Vec<(UpdateInput, String)> = self
            .inputs
            .iter()
            .filter(|i| keep(i.input.id.as_str()))
            .map(|i| {
                let uri = self.get_input_text(i);
                (i.clone(), uri)
            })
            .collect();

        if pending.is_empty() {
            return;
        }

        // One GraphQL POST resolves every github.com lookup in
        // `pending`. The REST path stays intact for non-github
        // forges and as a fallback if the warm step fails (anonymous
        // run, partial errors, transport hiccup), so worst case is a
        // wasted POST plus the existing per-input REST round trips.
        let github_lookups = build_github_batch_lookups(&pending);
        // Threshold of 2: a single-input batch trades a REST GET for
        // a GraphQL POST of the same wall-clock cost without any
        // overlap dividend, so the parallel-fetch path keeps it.
        // Two or more inputs are where the round-trip count actually
        // collapses.
        if github_lookups.len() >= 2
            && let Err(e) = self.client.batch_warm_github(&github_lookups)
        {
            tracing::debug!(
                "GraphQL batch warm failed; falling back to REST per input: {}",
                e
            );
        }

        let results = parallel_fetch(&self.client, pending, init);

        for (input, plan) in results {
            let Some(plan) = plan else { continue };
            if Self::print_update_status(
                input.input.id.as_str(),
                &plan.previous_ref,
                &plan.final_change,
            ) {
                self.update_input(input, &plan.updated_uri);
            }
        }
    }

    /// Current source after all queued edits.
    pub fn get_changes(&self) -> String {
        self.text.to_string()
    }

    fn get_input_text(&self, input: &UpdateInput) -> String {
        let (start, end) = self.url_char_range(input);
        self.text.slice(start..end).to_string()
    }

    /// Rewrite `input`'s URL to pin it to `rev`.
    pub(crate) fn change_input_to_rev(&mut self, input: &UpdateInput, rev: &str) {
        let uri = self.get_input_text(input);
        match uri.parse::<FlakeRef>() {
            Ok(parsed) => {
                let updated = parsed.pin_to_rev(rev.into()).into_uri();
                self.update_input(input.clone(), &updated);
            }
            Err(e) => {
                tracing::error!("Error while changing input: {}", e);
            }
        }
    }
    fn remove_ref_and_rev(&mut self, input: &UpdateInput) {
        let uri = self.get_input_text(input);
        match uri.parse::<FlakeRef>() {
            Ok(mut parsed) => {
                if parsed.ref_kind() == RefKind::None {
                    return;
                }
                parsed.set_ref(None);
                parsed.set_rev(None);
                self.update_input(input.clone(), &parsed.into_uri());
            }
            Err(e) => {
                tracing::error!("Error while changing input: {}", e);
            }
        }
    }
    // Sort by source range so multi-edit passes stay aligned with `offset`.
    fn sort(&mut self) {
        self.inputs.sort();
    }
    fn update_input(&mut self, input: UpdateInput, change: &str) {
        let (start, end) = self.url_char_range(&input);
        let previous_len = (end - start) as i32;
        self.text.remove(start..end);
        self.text.insert(start, change);
        self.offset += change.chars().count() as i32 - previous_len;
    }
}

/// Wrapper that lets [`Updater`] sort inputs by source position.
#[derive(Debug, Clone)]
pub(crate) struct UpdateInput {
    input: Input,
    /// Char index of the first URL character (inside the quotes) in the
    /// original, unmodified text.
    url_start: usize,
    /// Char index one past the last URL character in the original text.
    url_end: usize,
}

impl Ord for UpdateInput {
    fn cmp(&self, other: &Self) -> Ordering {
        self.url_start.cmp(&other.url_start)
    }
}

impl PartialEq for UpdateInput {
    fn eq(&self, other: &Self) -> bool {
        self.url_start == other.url_start
    }
}

impl PartialOrd for UpdateInput {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Eq for UpdateInput {}

/// Results come back in the same order as `pending`, which the
/// caller is expected to keep sorted by source position so the
/// subsequent edit phase can walk results left-to-right without a
/// second sort.
///
/// Workers re-use one [`ForgeClient`], so fan-out also amortises
/// TCP / TLS over its connection pool rather than running N
/// independent handshakes.
fn parallel_fetch(
    client: &ForgeClient,
    pending: Vec<(UpdateInput, String)>,
    init: bool,
) -> Vec<(UpdateInput, Option<UpdatePlan>)> {
    let n = pending.len();
    if n == 0 {
        return Vec::new();
    }
    let cap = std::cmp::min(n, FETCH_CONCURRENCY);

    // With nothing to overlap, skip the pool entirely; keeps the
    // single-input `update <id>` path on the calling thread.
    if cap <= 1 {
        let mut results = Vec::with_capacity(n);
        for (input, uri) in pending {
            let plan = compute_change(client, &uri, init);
            results.push((input, plan));
        }
        return results;
    }

    // Pop order is irrelevant: each work item carries its index so
    // results land in source-order `slots` regardless of which
    // worker happens to claim it.
    type WorkItem = (usize, UpdateInput, String);
    type ResultSlot = Mutex<Option<(UpdateInput, Option<UpdatePlan>)>>;
    let work: Mutex<Vec<WorkItem>> = Mutex::new(
        pending
            .into_iter()
            .enumerate()
            .map(|(i, (u, s))| (i, u, s))
            .collect(),
    );
    let slots: Vec<ResultSlot> = (0..n).map(|_| Mutex::new(None)).collect();

    std::thread::scope(|s| {
        for _ in 0..cap {
            let work = &work;
            let slots = &slots;
            s.spawn(move || {
                loop {
                    let next = work.lock().expect("fetch work queue poisoned").pop();
                    let Some((idx, input, uri)) = next else { break };
                    let plan = compute_change(client, &uri, init);
                    *slots[idx].lock().expect("fetch result slot poisoned") = Some((input, plan));
                }
            });
        }
    });

    slots
        .into_iter()
        .map(|m| {
            m.into_inner()
                .expect("fetch result slot poisoned")
                .expect("scope returned with an unfilled fetch slot")
        })
        .collect()
}

/// Plan one [`BatchLookup`] per github.com input in `pending`.
///
/// Inputs whose canonical domain is not `github.com`, or whose ref
/// is unpinned, unstable, or unrecognized, are skipped: those inputs
/// fall through to the REST path uncached. The returned list keeps
/// `pending`'s order so aliases in the GraphQL document line up
/// 1:1 with source-ordered inputs, which keeps debug logs and
/// breakpoints readable.
fn build_github_batch_lookups(pending: &[(UpdateInput, String)]) -> Vec<BatchLookup> {
    let mut lookups = Vec::new();
    for (_, uri) in pending {
        let Ok(parsed) = uri.parse::<FlakeRef>() else {
            continue;
        };
        // canonical_domain(None) == "github.com": shorthand inputs
        // route through the github cache slot, so they batch the same
        // way explicit `https://github.com/...` inputs do.
        let canonical = match parsed.domain() {
            None => "github.com",
            Some(d) => d,
        };
        if canonical != "github.com" {
            continue;
        }
        let (Some(owner), Some(repo)) = (parsed.owner(), parsed.repo()) else {
            continue;
        };
        match detect_strategy(owner, repo) {
            UpdateStrategy::SemverTags => {
                lookups.push(BatchLookup::Tags {
                    owner: owner.to_string(),
                    repo: repo.to_string(),
                });
            }
            UpdateStrategy::NixpkgsChannel
            | UpdateStrategy::HomeManagerChannel
            | UpdateStrategy::NixDarwinChannel => {
                let current_ref = parsed.ref_or_rev().unwrap_or_default();
                if current_ref.is_empty() {
                    continue;
                }
                let channel = parse_channel_ref(current_ref);
                let (Some(prefix), Some(current_version)) = (channel.prefix(), channel.version())
                else {
                    continue;
                };
                let candidates = channel_probe_candidates(prefix, current_version);
                lookups.push(BatchLookup::ChannelCandidates {
                    owner: owner.to_string(),
                    repo: repo.to_string(),
                    prefix: prefix.to_string(),
                    candidates,
                });
            }
        }
    }
    lookups
}

/// Resolve the new URI for a single input.
///
/// Pure with respect to the [`Updater`] state: takes only
/// `&ForgeClient` plus the snapshotted URI text, so it is safe to
/// run on a worker thread without aliasing `Updater::text` /
/// `Updater::offset`. Per-input forge errors are logged via
/// `tracing` and returned as `None`, so one flaky input never
/// aborts the rest of the update run.
fn compute_change(client: &ForgeClient, uri: &str, init: bool) -> Option<UpdatePlan> {
    // `FlakeRef` exposes no owner/repo for tarball-archive URLs, so
    // recover the parts from the URL string and resolve them here.
    if let Some(archive) = ArchiveUrl::parse(uri) {
        return compute_archive_change(client, &archive, init);
    }

    let parsed = match uri.parse::<FlakeRef>() {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("Failed to parse URI: {}", e);
            return None;
        }
    };

    let owner = match parsed.owner() {
        Some(o) => o.to_owned(),
        None => {
            tracing::debug!("Skipping input without owner");
            return None;
        }
    };
    let repo = match parsed.repo() {
        Some(r) => r.to_owned(),
        None => {
            tracing::debug!("Skipping input without repo");
            return None;
        }
    };

    let strategy = detect_strategy(&owner, &repo);
    tracing::debug!("Update strategy for {}/{}: {:?}", owner, repo, strategy);

    match strategy {
        UpdateStrategy::NixpkgsChannel
        | UpdateStrategy::HomeManagerChannel
        | UpdateStrategy::NixDarwinChannel => {
            compute_channel_change(client, &parsed, &owner, &repo)
        }
        UpdateStrategy::SemverTags => {
            compute_semver_change(client, uri, &parsed, &owner, &repo, init)
        }
    }
}

/// Resolve the new URI for a tarball-archive input
/// (`<scheme>://<host>/<owner>/<repo>/archive/<ref>.<ext>`).
///
/// Routes on the ref token, not [`detect_strategy`]: a channel ref
/// resolves via [`find_latest_channel`], a semver ref via
/// [`ForgeClient::list_tags`], anything else returns `None`. The host
/// is passed through as the forge domain.
fn compute_archive_change(
    client: &ForgeClient,
    archive: &ArchiveUrl,
    init: bool,
) -> Option<UpdatePlan> {
    let owner = archive.owner();
    let repo = archive.repo();
    let host = archive.host();
    let current_ref = archive.ref_token();
    let prefix = archive.ref_prefix_str();

    if matches!(parse_channel_ref(current_ref), ChannelType::Unknown) {
        let parsed_ref = parse_ref(current_ref, false);
        if !init && semver::Version::parse(&parsed_ref.normalized_for_semver).is_err() {
            tracing::debug!(
                "Skipping archive input {}/{}: ref {} is neither a channel nor semver",
                owner,
                repo,
                current_ref
            );
            return None;
        }

        let tags = match client.list_tags(owner, repo, Some(host)) {
            Ok(t) => t,
            Err(e) => {
                tracing::error!(
                    "Failed to fetch tags for archive input {}/{} on {}: {}",
                    owner,
                    repo,
                    host,
                    e
                );
                return None;
            }
        };
        let latest = match tags.get_latest_tag() {
            Some(c) => c,
            None => {
                tracing::error!(
                    "Could not find latest version for archive input {}/{}",
                    owner,
                    repo
                );
                return None;
            }
        };

        if !init && is_downgrade(current_ref, &latest) {
            tracing::warn!(
                "Refusing to downgrade archive input {}/{} from {} to {}",
                owner,
                repo,
                current_ref,
                latest
            );
            eprintln!(
                "Warning: skipping {}/{}: latest tag {} is older than the current pin {}.",
                owner, repo, latest, current_ref
            );
            return None;
        }

        Some(UpdatePlan {
            previous_ref: format!("{prefix}{current_ref}"),
            final_change: format!("{prefix}{latest}"),
            updated_uri: archive.with_ref(&latest),
        })
    } else {
        let latest = match find_latest_channel(client, current_ref, owner, repo, Some(host)) {
            Ok(Some(latest)) => latest,
            Ok(None) => return None,
            Err(e) => {
                tracing::error!(
                    "Failed to resolve latest channel for archive input {}/{}: {}",
                    owner,
                    repo,
                    e
                );
                return None;
            }
        };

        Some(UpdatePlan {
            previous_ref: format!("{prefix}{current_ref}"),
            final_change: format!("{prefix}{latest}"),
            updated_uri: archive.with_ref(&latest),
        })
    }
}

/// Resolve the new URI for a channel-strategy input (nixpkgs,
/// home-manager, nix-darwin). Returns `None` when the input is
/// unpinned, the ref is unstable, or the forge probe fails. The
/// failure case logs through `tracing` so a flaky channel does not
/// abort the rest of the run. Preserves a `refs/heads/` prefix on
/// the final ref iff the input already carried one.
fn compute_channel_change(
    client: &ForgeClient,
    parsed: &FlakeRef,
    owner: &str,
    repo: &str,
) -> Option<UpdatePlan> {
    let domain = parsed.domain();
    let current_ref = parsed.ref_or_rev().unwrap_or_default().to_owned();

    if current_ref.is_empty() {
        tracing::debug!("Skipping unpinned channel input: {}/{}", owner, repo);
        return None;
    }

    let has_refs_heads_prefix = current_ref.starts_with("refs/heads/");

    let latest = match find_latest_channel(client, &current_ref, owner, repo, domain) {
        Ok(Some(latest)) => latest,
        Ok(None) => return None,
        Err(e) => {
            tracing::error!(
                "Failed to resolve latest channel for {}/{}: {}",
                owner,
                repo,
                e
            );
            return None;
        }
    };

    let final_ref = if has_refs_heads_prefix {
        format!("refs/heads/{}", latest)
    } else {
        latest.clone()
    };
    let updated_uri = parsed.clone().with_ref(Some(final_ref.clone())).into_uri();

    Some(UpdatePlan {
        previous_ref: current_ref,
        final_change: final_ref,
        updated_uri,
    })
}

/// Resolve the new URI for a semver-strategy input. Returns `None`
/// when the current ref does not parse as semver and `init` is off
/// (so non-semver pins aren't silently overwritten), or when the
/// tag listing fails. Preserves a `refs/tags/` prefix on the final
/// ref iff the input already carried one.
fn compute_semver_change(
    client: &ForgeClient,
    uri: &str,
    parsed: &FlakeRef,
    owner: &str,
    repo: &str,
    init: bool,
) -> Option<UpdatePlan> {
    let is_git = is_git_url(uri);
    let maybe_version = parsed.ref_or_rev().unwrap_or_default();
    let parsed_ref = parse_ref(maybe_version, init);

    if !init && let Err(e) = semver::Version::parse(&parsed_ref.normalized_for_semver) {
        tracing::debug!("Skip non semver version: {}: {}", maybe_version, e);
        return None;
    }

    let tags = if is_git {
        let domain = parsed.domain()?.to_owned();
        match client.list_tags(owner, repo, Some(&domain)) {
            Ok(t) => t,
            Err(_) => {
                tracing::error!("Failed to fetch tags for {}/{} on {}", owner, repo, domain);
                return None;
            }
        }
    } else {
        match client.list_tags(owner, repo, None) {
            Ok(t) => t,
            Err(_) => {
                tracing::error!("Failed to fetch tags for {}/{}", owner, repo);
                return None;
            }
        }
    };

    let change = match tags.get_latest_tag() {
        Some(c) => c,
        None => {
            tracing::error!("Could not find latest version for {}/{}", owner, repo);
            return None;
        }
    };

    if !init && is_downgrade(maybe_version, &change) {
        tracing::warn!(
            "Refusing to downgrade {}/{} from {} to {}",
            owner,
            repo,
            maybe_version,
            change
        );
        eprintln!(
            "Warning: skipping {}/{}: latest tag {} is older than the current pin {}.",
            owner, repo, change, maybe_version
        );
        return None;
    }

    let final_change = if parsed_ref.has_refs_tags_prefix {
        format!("refs/tags/{}", change)
    } else {
        change.clone()
    };
    let updated_uri = parsed
        .clone()
        .with_ref(Some(final_change.clone()))
        .into_uri();

    Some(UpdatePlan {
        previous_ref: parsed_ref.previous_ref,
        final_change,
        updated_uri,
    })
}
