use nix_uri::FlakeRef;
use ropey::Rope;
use std::cmp::Ordering;

use crate::edit::InputMap;
use crate::input::Input;
use crate::uri::{extract_domain_owner_repo, is_git_url};
use crate::version::parse_ref;

#[derive(Default, Debug)]
pub struct Updater {
    text: Rope,
    inputs: Vec<UpdateInput>,
    // Keeps track of offset for changing multiple inputs on a single pass.
    offset: i32,
}

#[derive(Debug)]
struct RefSource {
    maybe_version: String,
    is_params: bool,
    is_github: bool,
}

enum UpdateTarget {
    GitUrl {
        uri: String,
        owner: String,
        repo: String,
        domain: String,
        parsed_ref: crate::version::ParsedRef,
    },
    ForgeRef {
        parsed: Box<FlakeRef>,
        owner: String,
        repo: String,
        ref_source: RefSource,
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
    fn ref_source_from_uri(uri: &str, parsed: Option<&FlakeRef>) -> RefSource {
        if is_git_url(uri) {
            let mut maybe_version = String::new();
            let mut is_params = false;

            if let Some(query) = uri.split('?').nth(1) {
                for param in query.split('&') {
                    if let Some(ref_value) = param.strip_prefix("ref=") {
                        maybe_version = ref_value.to_string();
                        is_params = true;
                        break;
                    }
                }
            }

            return RefSource {
                maybe_version,
                is_params,
                is_github: false,
            };
        }

        let parsed = match parsed {
            Some(parsed) => parsed,
            None => {
                return RefSource {
                    maybe_version: String::new(),
                    is_params: false,
                    is_github: false,
                };
            }
        };

        let mut maybe_version = String::new();
        let is_github = matches!(&parsed.r#type, nix_uri::FlakeRefType::GitHub { .. });

        if let nix_uri::FlakeRefType::GitHub {
            ref_or_rev: Some(ref_or_rev),
            ..
        } = &parsed.r#type
        {
            maybe_version = ref_or_rev.into();
        }

        let is_params = parsed.params.get_ref().is_some();
        if let Some(r#ref) = &parsed.params.get_ref() {
            maybe_version = r#ref.to_string();
        }

        RefSource {
            maybe_version,
            is_params,
            is_github,
        }
    }

    fn parse_update_target(&self, input: &UpdateInput, init: bool) -> Option<UpdateTarget> {
        let uri = self.get_input_text(input);
        let is_git_url = is_git_url(&uri);

        if is_git_url {
            // Extract the location part (before query parameters)
            let location = uri
                .strip_prefix("git+https://")
                .or_else(|| uri.strip_prefix("git+http://"))?;
            let location = location.split('?').next().unwrap_or(location);
            let (domain, owner, repo) = extract_domain_owner_repo(location)?;

            let ref_source = Self::ref_source_from_uri(&uri, None);
            let parsed_ref = parse_ref(&ref_source.maybe_version, init);

            if !init {
                if let Err(e) = semver::Version::parse(&parsed_ref.normalized_for_semver) {
                    tracing::debug!(
                        "Skip non semver version: {}: {}",
                        ref_source.maybe_version,
                        e
                    );
                    return None;
                }
            }

            return Some(UpdateTarget::GitUrl {
                uri,
                owner,
                repo,
                domain,
                parsed_ref,
            });
        }

        let parsed = match uri.parse::<FlakeRef>() {
            Ok(parsed) => parsed,
            Err(e) => {
                tracing::error!("{}", e);
                return None;
            }
        };

        let ref_source = Self::ref_source_from_uri(&uri, Some(&parsed));
        let parsed_ref = parse_ref(&ref_source.maybe_version, false);

        if !init {
            if let Err(e) = semver::Version::parse(&parsed_ref.normalized_for_semver) {
                tracing::debug!(
                    "Skip non semver version: {}: {}",
                    ref_source.maybe_version,
                    e
                );
                return None;
            }
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

        Some(UpdateTarget::ForgeRef {
            parsed: Box::new(parsed),
            owner,
            repo,
            ref_source,
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
        init: bool,
    ) {
        tags.sort();
        if let Some(change) = tags.get_latest_tag() {
            let (final_change, previous_ref, updated_uri) = match target {
                UpdateTarget::GitUrl {
                    uri, parsed_ref, ..
                } => {
                    let final_change = if parsed_ref.has_refs_tags_prefix {
                        format!("refs/tags/{}", change)
                    } else {
                        change.clone()
                    };
                    let base_url = uri.split('?').next().unwrap_or(uri);
                    let updated_uri = format!("{}?ref={}", base_url, final_change);
                    (final_change, parsed_ref.previous_ref.as_str(), updated_uri)
                }
                UpdateTarget::ForgeRef {
                    parsed,
                    ref_source,
                    parsed_ref,
                    ..
                } => {
                    let final_change = if parsed_ref.has_refs_tags_prefix {
                        format!("refs/tags/{}", change)
                    } else {
                        change.clone()
                    };
                    let mut parsed = parsed.clone();
                    if ref_source.is_github && !ref_source.is_params && !init {
                        let _ = parsed.r#type.ref_or_rev(Some(final_change.to_string()));
                    } else {
                        let _ = parsed.params.r#ref(Some(final_change.to_string()));
                    }
                    (
                        final_change,
                        parsed_ref.previous_ref.as_str(),
                        parsed.to_string(),
                    )
                }
            };

            if !Self::print_update_status(&input.input.id, previous_ref, &final_change) {
                return;
            }

            self.update_input(input.clone(), &updated_uri);
        } else {
            tracing::error!("Could not find latest version for Input: {:?}", input);
        }
    }
    pub fn new(text: Rope, map: InputMap) -> Self {
        let mut inputs = vec![];
        for (_id, input) in map {
            inputs.push(UpdateInput { input });
        }
        Self {
            inputs,
            text,
            offset: 0,
        }
    }
    /// TODO:
    /// Get an input based on it's id.
    fn get_index(&self, id: &str) -> usize {
        self.inputs.iter().position(|n| n.input.id == id).unwrap()
    }
    /// Pin an input based on it's id to a specific rev.
    pub fn pin_input_to_ref(&mut self, id: &str, rev: &str) {
        self.sort();
        let inputs = self.inputs.clone();
        if let Some(input) = inputs.get(self.get_index(id)) {
            tracing::debug!("Input: {:?}", input);
            self.change_input_to_rev(input, rev);
        }
    }
    /// Remove any ?ref= or ?rev= parameters from a specific input.
    pub fn unpin_input(&mut self, id: &str) {
        self.sort();
        let inputs = self.inputs.clone();
        if let Some(input) = inputs.get(self.get_index(id)) {
            tracing::debug!("Input: {:?}", input);
            self.remove_ref_and_rev(input);
        }
    }
    /// Update all inputs to a specific semver release,
    /// if a specific input is given, just update the single input.
    pub fn update_all_inputs_to_latest_semver(&mut self, id: Option<String>, init: bool) {
        self.sort();
        let inputs = self.inputs.clone();
        for input in inputs.iter() {
            if let Some(ref input_id) = id {
                if input.input.id == *input_id {
                    self.query_and_update_all_inputs(input, init);
                }
            } else {
                self.query_and_update_all_inputs(input, init);
            }
        }
    }
    pub fn get_changes(&self) -> String {
        self.text.to_string()
    }

    fn get_input_text(&self, input: &UpdateInput) -> String {
        self.text
            .slice(
                ((input.input.range.start as i32) + 1 + self.offset) as usize
                    ..((input.input.range.end as i32) + self.offset - 1) as usize,
            )
            .to_string()
    }

    /// Change a specific input to a specific rev.
    /// TODO: proper error handling
    pub fn change_input_to_rev(&mut self, input: &UpdateInput, rev: &str) {
        let uri = self.get_input_text(input);
        match uri.parse::<FlakeRef>() {
            Ok(mut parsed) => {
                parsed.params.rev(Some(rev.into()));
                // TODO: check, if rev_or_ref is already set, then change that side.
                // let _ = parsed.r#type.ref_or_rev(Some(rev.into()));
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
                let has_ref = parsed.params.get_ref().is_some();
                let has_rev = parsed.params.get_rev().is_some();
                if !has_ref && !has_rev {
                    return;
                }
                parsed.params.r#ref(None);
                parsed.params.rev(None);
                self.update_input(input.clone(), &parsed.to_string());
            }
            Err(e) => {
                tracing::error!("Error while changing input: {}", e);
            }
        }
    }
    /// Query a forge api for the latest release and update, if necessary.
    pub fn query_and_update_all_inputs(&mut self, input: &UpdateInput, init: bool) {
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

    // Sort the entries, so that we can adjust multiple values together
    fn sort(&mut self) {
        self.inputs.sort();
    }
    fn update_input(&mut self, input: UpdateInput, change: &str) {
        self.text.remove(
            (input.input.range.start as i32 + 1 + self.offset) as usize
                ..(input.input.range.end as i32 - 1 + self.offset) as usize,
        );
        self.text.insert(
            (input.input.range.start as i32 + 1 + self.offset) as usize,
            change,
        );
        self.update_offset(input.clone(), change);
    }
    fn update_offset(&mut self, input: UpdateInput, change: &str) {
        let previous_len = input.input.range.end as i32 - input.input.range.start as i32 - 2;
        let len = change.len() as i32;
        let offset = len - previous_len;
        self.offset += offset;
    }
}

// Wrapper around  individual inputs
#[derive(Debug, Clone)]
pub struct UpdateInput {
    input: Input,
}

impl Ord for UpdateInput {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.input.range.start).cmp(&(other.input.range.start))
    }
}

impl PartialEq for UpdateInput {
    fn eq(&self, other: &Self) -> bool {
        self.input.range.start == other.input.range.start
    }
}

impl PartialOrd for UpdateInput {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Eq for UpdateInput {}
