use std::collections::{BTreeMap, HashMap};

use crate::change::Change;
use crate::error::Error;
use crate::input::{Follows, Input};
use crate::validate;
use crate::walk::{Walker, toggle};

pub struct FlakeEdit {
    walker: Walker,
}

#[derive(Default, Debug)]
pub enum Outputs {
    #[default]
    None,
    Multiple(Vec<String>),
    Any(Vec<String>),
}

pub type InputMap = HashMap<String, Input>;

/// Sorted input ids from `inputs`.
pub fn sorted_input_ids(inputs: &InputMap) -> Vec<&String> {
    let mut keys: Vec<_> = inputs.keys().collect();
    keys.sort();
    keys
}

#[derive(Default, Debug)]
pub enum OutputChange {
    #[default]
    None,
    Add(String),
    Remove(String),
}

/// Toggle surface of one input: its active url and the stored alternates
/// adjacent to it, in file order.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ToggleState {
    /// The currently active url.
    pub active: String,
    /// Urls of the commented alternates next to the active binding.
    pub alternates: Vec<String>,
}

/// Result of applying a [`Change`].
///
/// `text` is the new flake source, or `None` for a no-op (e.g. an
/// already-existing follows declaration).
#[derive(Debug, Default)]
pub struct ApplyOutcome {
    pub text: Option<String>,
}

impl FlakeEdit {
    pub fn from_text(stream: &str) -> Result<Self, Error> {
        let parsed = validate::ParsedSource::new(stream);
        let validation = validate::validate_parsed(&parsed);
        if validation.has_errors() {
            return Err(Error::Validation(validation.errors));
        }

        let walker = Walker::from_root(parsed.syntax);
        Ok(Self { walker })
    }

    /// Wrap an already-parsed `flake.nix` syntax tree, skipping the parse and
    /// validation that [`Self::from_text`] runs. Reserved for the auto-follow
    /// apply loop, where each iteration validates its result and feeds the
    /// resulting parse back to the next iteration's walker.
    #[cfg(feature = "application")]
    pub(crate) fn from_syntax(syntax: rnix::SyntaxNode) -> Self {
        Self {
            walker: Walker::from_root(syntax),
        }
    }

    pub fn source_text(&self) -> String {
        self.walker.root.to_string()
    }

    pub fn curr_list(&self) -> &InputMap {
        &self.walker.inputs
    }

    /// Re-walk the source and return the freshly populated input map. Use
    /// [`Self::curr_list`] to read the cached map without re-walking.
    pub fn list(&mut self) -> &InputMap {
        self.walker.inputs.clear();
        // Walk returns Ok(None) when no changes are made (expected for listing)
        assert!(self.walker.walk(&Change::None).ok().flatten().is_none());
        &self.walker.inputs
    }
    /// Apply `change` and return the resulting [`ApplyOutcome`].
    ///
    /// Some edits require multiple walker passes. This method drives them all.
    /// A fatal validation failure surfaces as
    /// [`Error::Validation`].
    ///
    /// # Errors
    ///
    /// Returns [`Error`] if the underlying walker fails or the change
    /// is rejected (e.g. [`Error::DuplicateInput`],
    /// [`Error::InputNotFound`]).
    pub fn apply_change(&mut self, change: Change) -> Result<ApplyOutcome, Error> {
        let text = self.apply_change_text(change)?;
        Ok(ApplyOutcome { text })
    }

    fn apply_change_text(&mut self, change: Change) -> Result<Option<String>, Error> {
        match change {
            Change::None => Ok(None),
            Change::Add { .. } => self.apply_add(change),
            Change::Remove { .. } => self.apply_remove(change),
            Change::Follows { .. } => self.apply_follows(change),
            Change::Change { .. } => self.apply_change_uri(change),
            Change::Toggle { .. } => self.apply_toggle(change),
            Change::ToggleRemove { .. } => self.apply_toggle_remove(change),
        }
    }

    /// The `Change::Add` path is two-shot: a first walk attempts to insert
    /// inside an existing `inputs = { ... }` block, and only if that returns
    /// `None` (no such block) does the walker re-run with `add_toplevel`
    /// flipped on to synthesize one. Outputs-lambda extension piggy-backs on
    /// the first walk because it must observe the post-insert syntax tree.
    fn apply_add(&mut self, change: Change) -> Result<Option<String>, Error> {
        if let Some(input_id) = change.id() {
            self.ensure_inputs_populated()?;

            let input_id_string = input_id.input().as_str().to_string();
            if self.walker.inputs.contains_key(&input_id_string) {
                return Err(Error::DuplicateInput(input_id_string));
            }
        }

        if let Some(maybe_changed_node) = self.walker.walk(&change.clone())? {
            let outputs = self.walker.list_outputs()?;
            match outputs {
                Outputs::Multiple(out) => {
                    let id = change.id().unwrap().input().as_str().to_string();
                    if !out.contains(&id) {
                        self.walker.root = maybe_changed_node.clone();
                        if let Some(maybe_changed_node) =
                            self.walker.change_outputs(OutputChange::Add(id))?
                        {
                            return Ok(Some(maybe_changed_node.to_string()));
                        }
                    }
                }
                Outputs::None | Outputs::Any(_) => {}
            }
            Ok(Some(maybe_changed_node.to_string()))
        } else {
            self.walker.add_toplevel = true;
            let maybe_changed_node = self.walker.walk(&change)?;
            Ok(maybe_changed_node.map(|n| n.to_string()))
        }
    }

    /// `Change::Remove` runs the walker in a fixed-point loop because a single
    /// input can be spelled across multiple flat declarations
    /// (`inputs.foo.url = ...; inputs.foo.flake = false;`); each walk strips
    /// one occurrence. The post-loop outputs-lambda strip and orphan-follows
    /// scrub only run for a top-level remove, since a depth-N follows id
    /// shares its first segment with a still-present input and running the
    /// cleanup there would strip that input from the outputs lambda.
    fn apply_remove(&mut self, change: Change) -> Result<Option<String>, Error> {
        self.ensure_inputs_populated()?;

        let id = change.id().unwrap();
        let is_toplevel_remove = id.follows().is_none();
        let removed_id = id.input().as_str().to_string();

        let mut res = None;
        while let Some(changed_node) = self.walker.walk(&change)? {
            if res == Some(changed_node.clone()) {
                break;
            }
            res = Some(changed_node.clone());
            self.walker.root = changed_node.clone();
        }

        if is_toplevel_remove {
            let outputs = self.walker.list_outputs()?;
            match outputs {
                Outputs::Multiple(out) | Outputs::Any(out) => {
                    if out.contains(&removed_id)
                        && let Some(changed_node) = self
                            .walker
                            .change_outputs(OutputChange::Remove(removed_id.clone()))?
                    {
                        res = Some(changed_node.clone());
                        self.walker.root = changed_node.clone();
                    }
                }
                Outputs::None => {}
            }

            let orphaned_follows = self.collect_orphaned_follows(&removed_id);
            for orphan_change in orphaned_follows {
                while let Some(changed_node) = self.walker.walk(&orphan_change)? {
                    if res == Some(changed_node.clone()) {
                        break;
                    }
                    res = Some(changed_node.clone());
                    self.walker.root = changed_node.clone();
                }
            }
        }

        Ok(res.map(|n| n.to_string()))
    }

    /// A `Change::Follows` whose parent input is missing is a hard error
    /// rather than a no-op so the caller learns about the typo; the parent
    /// check runs before the walk because the walker would silently produce
    /// no edit otherwise.
    fn apply_follows(&mut self, change: Change) -> Result<Option<String>, Error> {
        let Change::Follows { ref input, .. } = change else {
            unreachable!("apply_follows dispatched only for Change::Follows");
        };

        self.ensure_inputs_populated()?;

        let parent_id = input.input().as_str();
        if !self.walker.inputs.contains_key(parent_id) {
            return Err(Error::InputNotFound(parent_id.to_string()));
        }

        Ok(self.walker.walk(&change)?.map(|n| n.to_string()))
    }

    /// The presence check exists because `walker.walk` produces no edit at
    /// all when its `Change::Change` target is missing, so without surfacing
    /// `InputNotFound` here a typo would silently report success. The
    /// `Option<ChangeId>` is honored rather than asserted: a `Change::Change`
    /// with `id == None` is a no-op the walker handles without consulting
    /// the input map.
    fn apply_change_uri(&mut self, change: Change) -> Result<Option<String>, Error> {
        if let Some(input_id) = change.id() {
            self.ensure_inputs_populated()?;

            let input_id_string = input_id.input().as_str().to_string();
            if !self.walker.inputs.contains_key(&input_id_string) {
                return Err(Error::InputNotFound(input_id_string));
            }
        }

        Ok(self.walker.walk(&change)?.map(|n| n.to_string()))
    }

    /// A `Change::Toggle` edits through [`crate::walk::toggle`] directly
    /// rather than the traversal in `walk`: the url binding is found via
    /// the range recorded on the input, and the flip or synthesis is a
    /// local green-tree rewrite around it.
    fn apply_toggle(&mut self, change: Change) -> Result<Option<String>, Error> {
        let Change::Toggle { id, uri, .. } = change else {
            unreachable!("apply_toggle dispatched only for Change::Toggle");
        };

        self.ensure_inputs_populated()?;

        let id_str = id.input().as_str().to_string();
        let Some(input) = self.walker.inputs.get(&id_str) else {
            return Err(Error::InputNotFound(id_str));
        };
        let Some(binding) = toggle::url_binding(&self.walker.root, input) else {
            return Err(Error::NoUrlToToggle(id_str));
        };
        let parent = binding
            .parent()
            .expect("a url binding always sits inside an enclosing node");

        let alternates = toggle::alternates(&binding);
        if let Some(alternate) = alternates.iter().find(|a| a.url == uri) {
            return Ok(Some(toggle::flip(&parent, &binding, alternate).to_string()));
        }
        if input.url() == uri {
            // The url is already active and not stored as an alternate.
            // Synthesizing would write a duplicate line below it, so this
            // is a no-op.
            return Ok(None);
        }
        Ok(Some(
            toggle::synthesize(&parent, &binding, &uri).to_string(),
        ))
    }

    /// A `Change::ToggleRemove` deletes the resolved variant's line through
    /// [`crate::walk::toggle`]. Removing a stored alternate drops its
    /// comment and leaves the active url untouched; removing the active url
    /// activates `activate` in its place and drops the previously active
    /// line. A `uri` that is not stored on the input is a no-op.
    fn apply_toggle_remove(&mut self, change: Change) -> Result<Option<String>, Error> {
        let Change::ToggleRemove { id, uri, activate } = change else {
            unreachable!("apply_toggle_remove dispatched only for Change::ToggleRemove");
        };

        self.ensure_inputs_populated()?;

        let id_str = id.input().as_str().to_string();
        let Some(input) = self.walker.inputs.get(&id_str) else {
            return Err(Error::InputNotFound(id_str));
        };
        let Some(binding) = toggle::url_binding(&self.walker.root, input) else {
            return Err(Error::NoUrlToToggle(id_str));
        };
        let parent = binding
            .parent()
            .expect("a url binding always sits inside an enclosing node");

        let alternates = toggle::alternates(&binding);
        if let Some(alternate) = alternates.iter().find(|a| a.url == uri) {
            return Ok(Some(
                toggle::remove_alternate(&parent, alternate).to_string(),
            ));
        }
        if input.url() == uri {
            let replacement = activate
                .and_then(|activate| alternates.into_iter().find(|a| a.url == activate))
                .ok_or(Error::RemoveActiveWithoutAlternate(id_str))?;
            return Ok(Some(
                toggle::flip_remove(&parent, &binding, &replacement).to_string(),
            ));
        }
        Ok(None)
    }

    /// Toggle states for every input with a url binding, keyed by input
    /// id. Inputs without one (follows-only inputs) are absent. An input
    /// is toggleable when its state lists at least one alternate.
    pub fn toggle_states(&mut self) -> Result<BTreeMap<String, ToggleState>, Error> {
        self.ensure_inputs_populated()?;
        let mut states = BTreeMap::new();
        for (id, input) in &self.walker.inputs {
            let Some(binding) = toggle::url_binding(&self.walker.root, input) else {
                continue;
            };
            let alternates = toggle::alternates(&binding)
                .into_iter()
                .map(|a| a.url)
                .collect();
            states.insert(
                id.clone(),
                ToggleState {
                    active: input.url().to_string(),
                    alternates,
                },
            );
        }
        Ok(states)
    }

    pub fn walker(&self) -> &Walker {
        &self.walker
    }

    /// Walk once if the inputs map is empty.
    fn ensure_inputs_populated(&mut self) -> Result<(), Error> {
        if self.walker.inputs.is_empty() {
            let _ = self.walker.walk(&Change::None)?;
        }
        Ok(())
    }

    /// Collect [`Change::Remove`]s for follows declarations whose target
    /// top-level segment matches `removed_id`.
    fn collect_orphaned_follows(&self, removed_id: &str) -> Vec<Change> {
        let mut orphaned = Vec::new();
        for (input_id, input) in &self.walker.inputs {
            for follows in input.follows() {
                if let Follows::Indirect {
                    path,
                    target: Some(target),
                } = follows
                {
                    // target is the RHS of `follows = "..."`. Match when its
                    // top-level segment is the removed input. Empty targets
                    // (`follows = ""`) have nothing to follow and cannot
                    // dangle.
                    if target.first().as_str() == removed_id {
                        let path_str = format!("{}.{}", input_id, path);
                        let Ok(change_id) = crate::change::ChangeId::parse(&path_str) else {
                            continue;
                        };
                        orphaned.push(Change::Remove {
                            ids: vec![change_id],
                        });
                    }
                }
            }
        }
        orphaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flake_with_nixpkgs_and_crane() -> &'static str {
        r#"{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
    crane = {
      url = "github:ipetkov/crane";
    };
  };
  outputs = { ... }: { };
}"#
    }

    #[test]
    fn none_is_noop() {
        let mut fe = FlakeEdit::from_text(flake_with_nixpkgs_and_crane()).unwrap();
        let outcome = fe.apply_change(Change::None).unwrap();
        assert!(outcome.text.is_none(), "Change::None must not produce text");
    }

    #[test]
    fn add_inserts_into_existing_inputs_block() {
        let flake = r#"{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
  };
  outputs = { ... }: { };
}"#;
        let mut fe = FlakeEdit::from_text(flake).unwrap();
        let change = Change::Add {
            id: Some(crate::change::ChangeId::parse("crane").unwrap()),
            uri: Some("github:ipetkov/crane".into()),
            flake: true,
        };
        let text = fe
            .apply_change(change)
            .expect("Add must succeed")
            .text
            .expect("Add must produce text");
        assert!(
            text.contains("crane.url = \"github:ipetkov/crane\""),
            "new input must render as a flat url assignment; got:\n{text}",
        );
    }

    #[test]
    fn add_synthesizes_inputs_block_when_absent() {
        // The first walk returns None when the source has no
        // `inputs = { ... }` block to insert into. apply_add then flips
        // `add_toplevel` and re-runs to synthesize one.
        let flake = r#"{
  outputs = { self, ... }: { };
}"#;
        let mut fe = FlakeEdit::from_text(flake).unwrap();
        let change = Change::Add {
            id: Some(crate::change::ChangeId::parse("nixpkgs").unwrap()),
            uri: Some("github:nixos/nixpkgs".into()),
            flake: true,
        };
        let text = fe
            .apply_change(change)
            .expect("Add must succeed")
            .text
            .expect("Add must produce text");
        assert!(
            text.contains("inputs.nixpkgs.url = \"github:nixos/nixpkgs\""),
            "synthesized toplevel form must use flat url assignment; got:\n{text}",
        );
    }

    #[test]
    fn add_duplicate_returns_duplicate_input_error() {
        let mut fe = FlakeEdit::from_text(flake_with_nixpkgs_and_crane()).unwrap();
        let change = Change::Add {
            id: Some(crate::change::ChangeId::parse("crane").unwrap()),
            uri: Some("github:ipetkov/crane".into()),
            flake: true,
        };
        let err = fe.apply_change(change).expect_err("duplicate must error");
        assert!(
            matches!(err, Error::DuplicateInput(ref id) if id == "crane"),
            "expected DuplicateInput(\"crane\"), got: {err:?}",
        );
    }

    #[test]
    fn remove_strips_existing_input() {
        let mut fe = FlakeEdit::from_text(flake_with_nixpkgs_and_crane()).unwrap();
        let change = Change::Remove {
            ids: vec![crate::change::ChangeId::parse("crane").unwrap()],
        };
        let text = fe
            .apply_change(change)
            .expect("Remove must succeed")
            .text
            .expect("Remove must produce text");
        assert!(
            !text.contains("crane"),
            "removed id must not appear; got:\n{text}"
        );
        assert!(text.contains("nixpkgs"), "untouched id must remain");
    }

    #[test]
    fn remove_scrubs_orphaned_follows_pointing_at_removed_input() {
        // Removing a top-level input must also strip any sibling input's
        // `follows = "<removed>"` declaration; apply_remove gates this
        // scrub on `is_toplevel_remove`, so a depth-N remove must NOT
        // trigger it.
        let flake = r#"{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
    crane = {
      url = "github:ipetkov/crane";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };
  outputs = { ... }: { };
}"#;
        let mut fe = FlakeEdit::from_text(flake).unwrap();
        let change = Change::Remove {
            ids: vec![crate::change::ChangeId::parse("nixpkgs").unwrap()],
        };
        let text = fe
            .apply_change(change)
            .expect("Remove must succeed")
            .text
            .expect("Remove must produce text");
        assert!(
            !text.contains("follows = \"nixpkgs\""),
            "orphaned follows must be scrubbed; got:\n{text}",
        );
        assert!(text.contains("crane"), "sibling input must remain");
    }

    #[test]
    fn change_uri_rewrites_existing_input() {
        let mut fe = FlakeEdit::from_text(flake_with_nixpkgs_and_crane()).unwrap();
        let change = Change::Change {
            id: Some(crate::change::ChangeId::parse("crane").unwrap()),
            uri: Some("github:ipetkov/crane/v0.20.0".into()),
        };
        let text = fe
            .apply_change(change)
            .expect("Change must succeed")
            .text
            .expect("Change must produce text");
        assert!(
            text.contains("github:ipetkov/crane/v0.20.0"),
            "new uri must be present; got:\n{text}",
        );
    }

    #[test]
    fn change_uri_missing_input_returns_input_not_found() {
        let mut fe = FlakeEdit::from_text(flake_with_nixpkgs_and_crane()).unwrap();
        let change = Change::Change {
            id: Some(crate::change::ChangeId::parse("does-not-exist").unwrap()),
            uri: Some("github:owner/repo".into()),
        };
        let err = fe
            .apply_change(change)
            .expect_err("missing input must error");
        assert!(
            matches!(err, Error::InputNotFound(ref id) if id == "does-not-exist"),
            "expected InputNotFound(\"does-not-exist\"), got: {err:?}",
        );
    }

    #[test]
    fn follows_missing_parent_returns_input_not_found() {
        let mut fe = FlakeEdit::from_text(flake_with_nixpkgs_and_crane()).unwrap();
        let change = Change::Follows {
            input: crate::change::ChangeId::parse("ghost.nixpkgs").unwrap(),
            target: crate::follows::AttrPath::parse("nixpkgs").unwrap(),
        };
        let err = fe
            .apply_change(change)
            .expect_err("missing parent must error");
        assert!(
            matches!(err, Error::InputNotFound(ref id) if id == "ghost"),
            "expected InputNotFound(\"ghost\"), got: {err:?}",
        );
    }

    #[test]
    fn already_follows_is_noop() {
        let flake = r#"{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
    crane = {
      url = "github:ipetkov/crane";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };
  outputs = { ... }: { };
}"#;
        let mut fe = FlakeEdit::from_text(flake).unwrap();
        let original = fe.source_text();
        let change = Change::Follows {
            input: crate::change::ChangeId::parse("crane.nixpkgs").unwrap(),
            target: crate::follows::AttrPath::parse("nixpkgs").unwrap(),
        };
        let result = fe.apply_change(change).unwrap();
        // Walker signals a no-op as either the unchanged text or `None`.
        // Both are acceptable here.
        if let Some(text) = result.text {
            assert_eq!(text, original, "text should be unchanged");
        }
    }

    #[test]
    fn new_follows_succeeds() {
        let flake = r#"{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
    crane = {
      url = "github:ipetkov/crane";
    };
  };
  outputs = { ... }: { };
}"#;
        let mut fe = FlakeEdit::from_text(flake).unwrap();
        let change = Change::Follows {
            input: crate::change::ChangeId::parse("crane.nixpkgs").unwrap(),
            target: crate::follows::AttrPath::parse("nixpkgs").unwrap(),
        };
        let result = fe.apply_change(change);
        assert!(result.is_ok(), "expected Ok, got: {:?}", result);
        let text = result.unwrap().text.unwrap();
        assert!(text.contains("inputs.nixpkgs.follows = \"nixpkgs\""));
    }

    #[test]
    fn follows_target_with_dots_renders_as_flat_string() {
        use crate::follows::{AttrPath, Segment};

        let flake = r#"{
  inputs = {
    "ghc-8.6.5-iohk".url = "github:input-output-hk/ghc";
    crane = {
      url = "github:ipetkov/crane";
    };
  };
  outputs = { ... }: { };
}"#;
        let mut fe = FlakeEdit::from_text(flake).unwrap();
        let target_seg = Segment::from_unquoted("ghc-8.6.5-iohk").unwrap();
        let change = Change::Follows {
            input: crate::change::ChangeId::parse("crane.\"ghc-8.6.5-iohk\"").unwrap(),
            target: AttrPath::new(target_seg),
        };
        let text = fe
            .apply_change(change)
            .expect("apply Change::Follows")
            .text
            .expect("walker must produce changed text");

        let expected = "inputs.\"ghc-8.6.5-iohk\".follows = \"ghc-8.6.5-iohk\";";
        assert!(
            text.contains(expected),
            "RHS must render as a flat Nix string, got:\n{text}",
        );
        assert!(
            !text.contains(r#""ghc-8."6"."#),
            "RHS must not contain segment-by-segment quoting, got:\n{text}",
        );
        assert!(
            !text.contains(r#"= ""ghc-8"#),
            "RHS must not double-quote the target, got:\n{text}",
        );
    }
}
