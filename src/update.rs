use nix_uri::{FlakeRef, RefLocation};
use ropey::Rope;
use std::cmp::Ordering;

use crate::channel::{UpdateStrategy, detect_strategy, find_latest_channel};
use crate::edit::InputMap;
use crate::input::Input;
use crate::uri::is_git_url;
use crate::version::parse_ref;

/// Rewrites flake.nix URIs for `update`, `pin`, and `unpin`.
#[derive(Default, Debug)]
pub struct Updater {
    text: Rope,
    inputs: Vec<UpdateInput>,
    /// Cumulative char delta from edits applied earlier in the current pass.
    /// Measured in *characters*, since ropey indexes by char.
    offset: i32,
}

enum UpdateTarget {
    GitUrl {
        parsed: Box<FlakeRef>,
        owner: String,
        repo: String,
        domain: String,
        parsed_ref: crate::version::ParsedRef,
    },
    ForgeRef {
        parsed: Box<FlakeRef>,
        owner: String,
        repo: String,
        parsed_ref: crate::version::ParsedRef,
    },
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
    fn parse_update_target(&self, input: &UpdateInput, init: bool) -> Option<UpdateTarget> {
        let uri = self.get_input_text(input);
        let is_git_url = is_git_url(&uri);

        let parsed = match uri.parse::<FlakeRef>() {
            Ok(parsed) => parsed,
            Err(e) => {
                tracing::error!("Failed to parse URI: {}", e);
                return None;
            }
        };

        let maybe_version = parsed.get_ref_or_rev().unwrap_or_default();
        let parsed_ref = parse_ref(&maybe_version, init);

        if !init && let Err(e) = semver::Version::parse(&parsed_ref.normalized_for_semver) {
            tracing::debug!("Skip non semver version: {}: {}", maybe_version, e);
            return None;
        }

        let owner = match parsed.r#type.get_owner() {
            Some(o) => o,
            None => {
                tracing::debug!("Skipping input without owner");
                return None;
            }
        };

        let repo = match parsed.r#type.get_repo() {
            Some(r) => r,
            None => {
                tracing::debug!("Skipping input without repo");
                return None;
            }
        };

        if is_git_url {
            let domain = parsed.r#type.get_domain()?;
            return Some(UpdateTarget::GitUrl {
                parsed: Box::new(parsed),
                owner,
                repo,
                domain,
                parsed_ref,
            });
        }

        Some(UpdateTarget::ForgeRef {
            parsed: Box::new(parsed),
            owner,
            repo,
            parsed_ref,
        })
    }

    fn fetch_tags(&self, target: &UpdateTarget) -> Option<crate::api::Tags> {
        match target {
            UpdateTarget::GitUrl {
                owner,
                repo,
                domain,
                ..
            } => match crate::api::get_tags(repo, owner, Some(domain)) {
                Ok(tags) => Some(tags),
                Err(_) => {
                    tracing::error!("Failed to fetch tags for {}/{} on {}", owner, repo, domain);
                    None
                }
            },
            UpdateTarget::ForgeRef { owner, repo, .. } => {
                match crate::api::get_tags(repo, owner, None) {
                    Ok(tags) => Some(tags),
                    Err(_) => {
                        tracing::error!("Failed to fetch tags for {}/{}", owner, repo);
                        None
                    }
                }
            }
        }
    }

    fn apply_update(
        &mut self,
        input: &UpdateInput,
        target: &UpdateTarget,
        mut tags: crate::api::Tags,
        _init: bool,
    ) {
        tags.sort();
        if let Some(change) = tags.get_latest_tag() {
            let (parsed, parsed_ref) = match target {
                UpdateTarget::GitUrl {
                    parsed, parsed_ref, ..
                } => (parsed, parsed_ref),
                UpdateTarget::ForgeRef {
                    parsed, parsed_ref, ..
                } => (parsed, parsed_ref),
            };

            let final_change = if parsed_ref.has_refs_tags_prefix {
                format!("refs/tags/{}", change)
            } else {
                change.clone()
            };

            // `set_ref` preserves whether the ref lives in the URL path or a
            // query parameter.
            let mut parsed = parsed.clone();
            let _ = parsed.set_ref(Some(final_change.clone()));
            let updated_uri = parsed.to_string();

            if !Self::print_update_status(
                input.input.id.as_str(),
                &parsed_ref.previous_ref,
                &final_change,
            ) {
                return;
            }

            self.update_input(input.clone(), &updated_uri);
        } else {
            tracing::error!("Could not find latest version for Input: {:?}", input);
        }
    }
    /// Build an [`Updater`] from `text` and the inputs in `map`. Inputs
    /// without an editable URL are skipped.
    pub fn new(text: Rope, map: InputMap) -> Self {
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

    /// Update inputs to the latest matching release.
    ///
    /// When `id` is `Some`, only that input is updated. Otherwise every
    /// input with an editable URL is processed.
    pub fn update_all_inputs_to_latest_semver(&mut self, id: Option<String>, init: bool) {
        self.sort();
        let inputs = self.inputs.clone();
        for input in inputs.iter() {
            if let Some(ref input_id) = id {
                if input.input.id.as_str() == input_id.as_str() {
                    self.query_and_update_all_inputs(input, init);
                }
            } else {
                self.query_and_update_all_inputs(input, init);
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
    pub fn change_input_to_rev(&mut self, input: &UpdateInput, rev: &str) {
        let uri = self.get_input_text(input);
        match uri.parse::<FlakeRef>() {
            Ok(mut parsed) => {
                // `set_rev` preserves whether the rev lives in the URL path
                // or a query parameter.
                let _ = parsed.set_rev(Some(rev.into()));
                self.update_input(input.clone(), &parsed.to_string());
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
                if parsed.ref_source_location() == RefLocation::None {
                    return;
                }
                // `set_ref`/`set_rev` handle both path-based and
                // query-parameter storage.
                let _ = parsed.set_ref(None);
                let _ = parsed.set_rev(None);
                self.update_input(input.clone(), &parsed.to_string());
            }
            Err(e) => {
                tracing::error!("Error while changing input: {}", e);
            }
        }
    }
    /// Query the forge API for `input`'s latest release and rewrite the URL
    /// when newer.
    pub fn query_and_update_all_inputs(&mut self, input: &UpdateInput, init: bool) {
        let uri = self.get_input_text(input);

        let parsed = match uri.parse::<FlakeRef>() {
            Ok(parsed) => parsed,
            Err(e) => {
                tracing::error!("Failed to parse URI: {}", e);
                return;
            }
        };

        let owner = match parsed.r#type.get_owner() {
            Some(o) => o,
            None => {
                tracing::debug!("Skipping input without owner");
                return;
            }
        };

        let repo = match parsed.r#type.get_repo() {
            Some(r) => r,
            None => {
                tracing::debug!("Skipping input without repo");
                return;
            }
        };

        let strategy = detect_strategy(&owner, &repo);
        tracing::debug!("Update strategy for {}/{}: {:?}", owner, repo, strategy);

        match strategy {
            UpdateStrategy::NixpkgsChannel
            | UpdateStrategy::HomeManagerChannel
            | UpdateStrategy::NixDarwinChannel => {
                self.update_channel_input(input, &parsed);
            }
            UpdateStrategy::SemverTags => {
                self.update_semver_input(input, init);
            }
        }
    }

    /// Update `input` using channel-based versioning (nixpkgs, home-manager,
    /// nix-darwin).
    fn update_channel_input(&mut self, input: &UpdateInput, parsed: &FlakeRef) {
        let owner = parsed.r#type.get_owner().unwrap();
        let repo = parsed.r#type.get_repo().unwrap();
        let domain = parsed.r#type.get_domain();

        let current_ref = parsed.get_ref_or_rev().unwrap_or_default();

        if current_ref.is_empty() {
            tracing::debug!(
                "Skipping unpinned channel input: {}",
                input.input.id.as_str()
            );
            return;
        }

        let has_refs_heads_prefix = current_ref.starts_with("refs/heads/");

        let latest = match find_latest_channel(&current_ref, &owner, &repo, domain.as_deref()) {
            Some(latest) => latest,
            // Either already on latest, unstable, or not a recognized channel
            None => return,
        };

        let final_ref = if has_refs_heads_prefix {
            format!("refs/heads/{}", latest)
        } else {
            latest.clone()
        };

        let mut parsed = parsed.clone();
        let _ = parsed.set_ref(Some(final_ref.clone()));
        let updated_uri = parsed.to_string();

        if Self::print_update_status(input.input.id.as_str(), &current_ref, &final_ref) {
            self.update_input(input.clone(), &updated_uri);
        }
    }

    /// Update `input` to the latest semver tag from its forge.
    fn update_semver_input(&mut self, input: &UpdateInput, init: bool) {
        let target = match self.parse_update_target(input, init) {
            Some(target) => target,
            None => return,
        };

        let tags = match self.fetch_tags(&target) {
            Some(tags) => tags,
            None => return,
        };

        self.apply_update(input, &target, tags, init);
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
pub struct UpdateInput {
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
