//! `flake-edit pin` and `flake-edit unpin`: freeze or release a
//! specific revision on an input.
//!
//! `pin` reads `flake.lock` to default the target rev when the user
//! does not supply one. `unpin`'s interactive picker filters to
//! inputs whose URL already carries a `ref_or_rev`.

use nix_uri::FlakeRef;

use crate::edit::{FlakeEdit, sorted_input_ids};

use super::super::editor::Editor;
use super::super::state::AppState;
use super::{CommandError, Result, interactive_single_select, load_flake_lock, updater};

pub fn pin(
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
    id: Option<String>,
    rev: Option<String>,
) -> Result<()> {
    let inputs = flake_edit.list().clone();
    let input_ids = sorted_input_ids(&inputs)
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();

    if let Some(id) = id {
        let lock = load_flake_lock(state).map_err(|e| CommandError::LockFileError {
            path: state
                .lock_file
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "flake.lock".to_string()),
            source: e,
        })?;
        let target_rev = if let Some(rev) = rev {
            rev
        } else {
            lock.rev_for(&id)
                .map_err(|_| CommandError::InputNotFound(id.clone()))?
        };
        let mut updater = updater(editor, inputs);
        updater
            .pin_input_to_ref(&id, &target_rev)
            .map_err(CommandError::InputNotPinnable)?;
        let change = updater.get_changes();
        editor.apply_or_diff(&change, state)?;
        if !state.diff {
            println!("Pinned input: {} to {}", id, target_rev);
        }
    } else if state.interactive {
        if input_ids.is_empty() {
            return Err(CommandError::NoInputs);
        }
        let lock = load_flake_lock(state).map_err(|e| CommandError::LockFileError {
            path: state
                .lock_file
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "flake.lock".to_string()),
            source: e,
        })?;

        interactive_single_select(
            editor,
            state,
            "Pin",
            "Select input",
            input_ids,
            |id| {
                let target_rev = lock
                    .rev_for(id)
                    .map_err(|_| CommandError::InputNotFound(id.to_string()))?;
                let mut updater = updater(editor, inputs.clone());
                updater
                    .pin_input_to_ref(id, &target_rev)
                    .map_err(CommandError::InputNotPinnable)?;
                Ok((updater.get_changes(), target_rev))
            },
            |id, target_rev| println!("Pinned input: {} to {}", id, target_rev),
        )?;
    } else {
        return Err(CommandError::NoId);
    }

    Ok(())
}

pub fn unpin(
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
    id: Option<String>,
) -> Result<()> {
    let inputs = flake_edit.list().clone();
    let input_ids = sorted_input_ids(&inputs)
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();

    if let Some(id) = id {
        let mut updater = updater(editor, inputs);
        updater
            .unpin_input(&id)
            .map_err(CommandError::InputNotPinnable)?;
        let change = updater.get_changes();
        editor.apply_or_diff(&change, state)?;
        if !state.diff {
            println!("Unpinned input: {}", id);
        }
    } else if state.interactive {
        let pinned_ids: Vec<String> = input_ids
            .into_iter()
            .filter(|id| {
                inputs[id]
                    .url()
                    .trim_matches('"')
                    .parse::<FlakeRef>()
                    .ok()
                    .and_then(|f| f.get_ref_or_rev())
                    .is_some_and(|v| !v.is_empty())
            })
            .collect();

        if pinned_ids.is_empty() {
            return Err(CommandError::NoInputs);
        }

        interactive_single_select(
            editor,
            state,
            "Unpin",
            "Select pinned input",
            pinned_ids,
            |id| {
                let mut updater = updater(editor, inputs.clone());
                updater
                    .unpin_input(id)
                    .map_err(CommandError::InputNotPinnable)?;
                Ok((updater.get_changes(), ()))
            },
            |id, ()| println!("Unpinned input: {}", id),
        )?;
    } else {
        return Err(CommandError::NoId);
    }

    Ok(())
}
