use nix_uri::{FlakeRef, RefLocation};
use ropey::Rope;
use std::cmp::Ordering;

use crate::edit::InputMap;
use crate::input::Input;
use crate::uri::is_git_url;
use crate::version::parse_ref;

#[derive(Default, Debug)]
pub struct Updater {
    text: Rope,
    inputs: Vec<UpdateInput>,
    // Keeps track of offset for changing multiple inputs on a single pass.
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

        if !init {
            if let Err(e) = semver::Version::parse(&parsed_ref.normalized_for_semver) {
                tracing::debug!("Skip non semver version: {}: {}", maybe_version, e);
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

            // set_ref() preserves storage location (path vs query param)
            let mut parsed = parsed.clone();
            let _ = parsed.set_ref(Some(final_change.clone()));
            let updated_uri = parsed.to_string();

            if !Self::print_update_status(&input.input.id, &parsed_ref.previous_ref, &final_change)
            {
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
    pub fn change_input_to_rev(&mut self, input: &UpdateInput, rev: &str) {
        let uri = self.get_input_text(input);
        match uri.parse::<FlakeRef>() {
            Ok(mut parsed) => {
                // set_rev() preserves storage location (path vs query param)
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
                // set_ref/set_rev handle both path-based and query param storage
                let _ = parsed.set_ref(None);
                let _ = parsed.set_rev(None);
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
