use nix_uri::FlakeRef;
use ropey::Rope;
use std::cmp::Ordering;

use crate::edit::InputMap;
use crate::input::Input;

#[derive(Default, Debug)]
pub struct Updater {
    text: Rope,
    inputs: Vec<UpdateInput>,
    // Keeps track of offset for changing multiple inputs on a single pass.
    offset: i32,
}

impl Updater {
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
    /// Query a forge api for the latest release and update, if necessary.
    pub fn query_and_update_all_inputs(&mut self, input: &UpdateInput, init: bool) {
        fn strip_until_char(s: &str, c: char) -> Option<String> {
            s.find(c).map(|index| s[index + 1..].to_string())
        }
        let uri = self
            .text
            .slice(
                ((input.input.range.start as i32) + 1 + self.offset) as usize
                    ..((input.input.range.end as i32) + self.offset - 1) as usize,
            )
            .to_string();
        match uri.parse::<FlakeRef>() {
            Ok(mut parsed) => {
                let mut maybe_version = String::new();
                let is_params = &parsed.params.get_ref().is_some();

                if let nix_uri::FlakeRefType::GitHub {
                    ref_or_rev: Some(ref_or_rev),
                    ..
                } = &parsed.r#type
                {
                    maybe_version = ref_or_rev.into();
                }

                if let Some(r#ref) = &parsed.params.get_ref() {
                    maybe_version = r#ref.to_string();
                }

                if let Some(normalized_version) = maybe_version.strip_prefix('v') {
                    maybe_version = normalized_version.to_string();
                }

                if let Some(normalized_version) = strip_until_char(&maybe_version, '-') {
                    maybe_version = normalized_version.to_string();
                }

                // If we init the version specifier we don't care if there was already a
                // correct semver specified, we automatically pin to the latest semver.
                if !init {
                    let _version = match semver::Version::parse(&maybe_version) {
                        Ok(v) => v,
                        Err(e) => {
                            tracing::debug!("Skip non semver version: {maybe_version}: {e}");
                            return;
                        }
                    };
                }

                let flake_type = &parsed.r#type;
                let mut tags = crate::api::get_tags(
                    &flake_type.get_owner().unwrap(),
                    &flake_type.get_repo().unwrap(),
                )
                .unwrap();

                tags.sort();
                if let Some(change) = tags.get_latest_tag() {
                    if *is_params || init {
                        let _ = parsed.params.r#ref(Some(change.to_string()));
                    } else {
                        let _ = parsed.r#type.ref_or_rev(Some(change.to_string()));
                    }
                } else {
                    tracing::error!("Could not find latest version for Input: {:?}", input);
                }

                self.update_input(input.clone(), &parsed.to_string());
            }
            Err(e) => {
                tracing::error!("{}", e);
            }
        }
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
