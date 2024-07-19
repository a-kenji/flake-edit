use nix_uri::FlakeRef;
use ropey::Rope;
use std::cmp::Ordering;

use crate::edit::InputMap;
use crate::input::Input;

#[derive(Default, Debug)]
pub struct Updater {
    text: Rope,
    inputs: Vec<UpdateInput>,
    offset: i32,
}

// TODO:
// - parse all uris for semver tags
// - query github if they are updated
// - update inputs that are out of date
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
    pub fn update(&mut self) {
        self.sort();
        let inputs = self.inputs.clone();
        for input in inputs.iter() {
            self.change_input(input);
        }
    }
    pub fn get_changes(&self) -> String {
        self.text.to_string()
    }

    pub fn change_input(&mut self, input: &UpdateInput) {
        let change = input.update();
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

                if let nix_uri::FlakeRefType::GitHub { ref_or_rev, .. } = &parsed.r#type {
                    if let Some(ref_or_rev) = ref_or_rev {
                        maybe_version = ref_or_rev.into();
                    }
                }
                if let Some(r#ref) = &parsed.params.get_ref() {
                    maybe_version = r#ref.to_string();
                }

                if let Some(normalized_version) = maybe_version.strip_prefix('v') {
                    maybe_version = normalized_version.to_string();
                }

                let _version = match semver::Version::parse(&maybe_version) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::debug!("Skip non semver version: {maybe_version}: {e}");
                        return;
                    }
                };

                let flake_type = &parsed.r#type;
                let mut tags = crate::api::get_tags(
                    &flake_type.get_owner().unwrap(),
                    &flake_type.get_repo().unwrap(),
                )
                .unwrap();

                tags.sort();
                let change = tags.get_latest_tag();

                if *is_params {
                    let _ = parsed.params.r#ref(Some(change.to_string()));
                } else {
                    let _ = parsed.r#type.ref_or_rev(Some(change.to_string()));
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
        // self.offset = 0;
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

impl UpdateInput {
    fn update(&self) -> &str {
        "1.3.4"
    }
}
