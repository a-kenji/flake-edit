//! `flake-edit update`: bump inputs to the latest semver match.
//!
//! Three modes: scripted single-input by ID, interactive
//! multi-select with current versions rendered for context, and a
//! non-interactive bump-everything path. `init` toggles whether
//! [`crate::forge::update::Updater`] seeds updates for inputs the lockfile
//! has not yet seen.

use nix_uri::FlakeRef;

use crate::edit::{FlakeEdit, sorted_input_ids};

use super::super::editor::Editor;
use super::super::state::AppState;
use super::{Error, Result, interactive_multi_select, updater};

pub fn update(
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
    id: Option<String>,
    init: bool,
) -> Result<()> {
    let inputs = flake_edit.list().clone();
    let input_ids = sorted_input_ids(&inputs)
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();

    if let Some(id) = id {
        let mut updater = updater(editor, inputs);
        updater.update_all_inputs_to_latest_semver(Some(id), init);
        let change = updater.get_changes();
        editor.apply_or_diff(&change, state)?;
    } else if state.interactive {
        if input_ids.is_empty() {
            return Err(Error::NoInputs);
        }

        let display_items: Vec<String> = input_ids
            .iter()
            .map(|id| {
                let input = &inputs[id];
                let version = input
                    .url()
                    .trim_matches('"')
                    .parse::<FlakeRef>()
                    .ok()
                    .and_then(|f| f.get_ref_or_rev());
                match version {
                    Some(v) if !v.is_empty() => format!("{} - {}", id, v),
                    _ => id.clone(),
                }
            })
            .collect();

        interactive_multi_select(
            editor,
            state,
            "Update",
            "Space select, U all, ^D diff",
            display_items,
            |selected| {
                // Strip the trailing version suffix from each display string.
                let ids: Vec<String> = selected
                    .iter()
                    .map(|s| s.split(" - ").next().unwrap_or(s).to_string())
                    .collect();
                let mut updater = updater(editor, inputs.clone());
                for id in &ids {
                    updater.update_all_inputs_to_latest_semver(Some(id.clone()), init);
                }
                updater.get_changes()
            },
        )?;
    } else {
        let mut updater = updater(editor, inputs);
        for id in &input_ids {
            updater.update_all_inputs_to_latest_semver(Some(id.clone()), init);
        }
        let change = updater.get_changes();
        editor.apply_or_diff(&change, state)?;
    }

    Ok(())
}
