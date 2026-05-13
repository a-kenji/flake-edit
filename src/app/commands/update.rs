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

    if let Some(id) = id {
        let mut updater = updater(editor, inputs);
        updater.update_inputs_to_latest_semver(&[id.as_str()], init);
        let change = updater.get_changes();
        editor.apply_or_diff(&change, state)?;
    } else if state.interactive {
        let input_ids = sorted_input_ids(&inputs)
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        if input_ids.is_empty() {
            return Err(Error::NoInputs);
        }

        let display_items: Vec<String> = input_ids
            .iter()
            .map(|id| {
                let input = &inputs[id];
                let parsed = input.url().trim_matches('"').parse::<FlakeRef>().ok();
                let version = parsed.as_ref().and_then(|f| f.ref_or_rev());
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
                let ids: Vec<&str> = selected
                    .iter()
                    .map(|s| s.split(" - ").next().unwrap_or(s))
                    .collect();
                let mut updater = updater(editor, inputs.clone());
                updater.update_inputs_to_latest_semver(&ids, init);
                updater.get_changes()
            },
        )?;
    } else {
        let mut updater = updater(editor, inputs);
        updater.update_all_to_latest_semver(init);
        let change = updater.get_changes();
        editor.apply_or_diff(&change, state)?;
    }

    Ok(())
}
