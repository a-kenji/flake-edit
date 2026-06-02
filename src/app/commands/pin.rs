//! `flake-edit pin` and `flake-edit unpin`: freeze or release a
//! specific revision on an input.
//!
//! `pin` reads `flake.lock` to default the target rev when the user
//! does not supply one. `unpin`'s interactive picker filters to
//! inputs whose URL already carries a `ref_or_rev`.

use nix_uri::FlakeRef;

use crate::edit::{FlakeEdit, sorted_input_ids};
use crate::follows::AttrPath;

use super::super::editor::Editor;
use super::super::state::AppState;
use super::{Error, Result, interactive_single_select, load_flake_lock, updater};

fn lock_path_display(state: &AppState) -> std::path::PathBuf {
    state
        .lock_file
        .clone()
        .unwrap_or_else(|| std::path::PathBuf::from("flake.lock"))
}

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
        let lock = load_flake_lock(state).map_err(|source| Error::LockFile {
            path: lock_path_display(state),
            source,
        })?;
        let target_rev = if let Some(rev) = rev {
            rev
        } else {
            let path = AttrPath::parse(&id).map_err(|source| Error::InvalidInputId {
                id: id.clone(),
                source,
            })?;
            lock.rev_for(&path)?
        };
        let mut updater = updater(editor, inputs);
        updater
            .pin_input_to_ref(&id, &target_rev)
            .map_err(|id| Error::InputNotPinnable { id })?;
        let change = updater.get_changes();
        editor.apply_or_diff(&change, state)?;
        if !state.diff {
            println!("Pinned input: {} to {}", id, target_rev);
        }
    } else if state.interactive {
        if input_ids.is_empty() {
            return Err(Error::NoInputs);
        }
        let lock = load_flake_lock(state).map_err(|source| Error::LockFile {
            path: lock_path_display(state),
            source,
        })?;

        interactive_single_select(
            editor,
            state,
            "Pin",
            "Select input",
            input_ids,
            |id| {
                let path = AttrPath::parse(id).map_err(|source| Error::InvalidInputId {
                    id: id.to_string(),
                    source,
                })?;
                let target_rev = lock.rev_for(&path)?;
                let mut updater = updater(editor, inputs.clone());
                updater
                    .pin_input_to_ref(id, &target_rev)
                    .map_err(|id| Error::InputNotPinnable { id })?;
                Ok((updater.get_changes(), target_rev))
            },
            |id, target_rev| println!("Pinned input: {} to {}", id, target_rev),
        )?;
    } else {
        return Err(Error::NoId);
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
            .map_err(|id| Error::InputNotPinnable { id })?;
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
                    .parse::<FlakeRef>()
                    .is_ok_and(|f| f.ref_kind() != nix_uri::RefKind::None)
            })
            .collect();

        if pinned_ids.is_empty() {
            return Err(Error::NoInputs);
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
                    .map_err(|id| Error::InputNotPinnable { id })?;
                Ok((updater.get_changes(), ()))
            },
            |id, ()| println!("Unpinned input: {}", id),
        )?;
    } else {
        return Err(Error::NoId);
    }

    Ok(())
}
