//! `flake-edit follow` and `flake-edit add-follow`: declare or
//! deduplicate `inputs.<id>.follows` edges.
//!
//! [`add_follow`] handles the scripted `<input> <target>` form and
//! the interactive picker fallback. [`auto`] runs the
//! auto-deduplication planner, applier, and batch driver.

pub mod auto;

use std::collections::HashSet;

use crate::change::{Change, ChangeId};
use crate::edit::{FlakeEdit, InputMap};
use crate::error::Error as FlakeError;
use crate::follows::AttrPath;
use crate::lock::NestedInput;
use crate::tui;

use super::super::editor::Editor;
use super::super::state::AppState;
use super::{Error, Result, apply_change, load_flake_lock};

pub(super) struct FollowContext {
    pub(super) nested_inputs: Vec<NestedInput>,
    pub(super) top_level_inputs: HashSet<String>,
    /// Full input map. Cycle detection needs URLs.
    pub(super) inputs: InputMap,
}

/// Loads the nested-input set from `flake.lock` and the top-level
/// input set from `flake.nix`, returning `Ok(None)` when either side
/// is empty so callers can short-circuit a no-op pass cleanly.
pub(super) fn load_follow_context(
    flake_edit: &mut FlakeEdit,
    state: &AppState,
) -> Result<Option<FollowContext>> {
    let nested_inputs: Vec<NestedInput> = match load_flake_lock(state) {
        Ok(lock) => lock.nested_inputs(),
        Err(e) => {
            let lock_path = state
                .lock_file
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "flake.lock".to_string());
            return Err(Error::LockFile {
                path: std::path::PathBuf::from(lock_path),
                source: e,
            });
        }
    };

    if nested_inputs.is_empty() {
        return Ok(None);
    }

    let inputs = flake_edit.list().clone();
    let top_level_inputs: HashSet<String> = inputs.keys().cloned().collect();

    // Inputless flakes (templates, NUR-style overlays, outputs-only
    // libraries) are legitimate; treat them as a no-op rather than an
    // error so batch invocations don't abort on the first one.
    // Symmetric with the `nested_inputs.is_empty()` short-circuit above.
    if top_level_inputs.is_empty() {
        return Ok(None);
    }

    Ok(Some(FollowContext {
        nested_inputs,
        top_level_inputs,
        inputs,
    }))
}

/// Adds a single follows declaration via the explicit
/// `<input> <target>` form, falling back to a TUI picker when both
/// positionals are absent and stdin is a terminal.
pub fn add_follow(
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
    input: Option<String>,
    target: Option<String>,
) -> Result<()> {
    let change = if let (Some(input_val), Some(target_val)) = (input.clone(), target) {
        let input_id = ChangeId::parse(&input_val).map_err(|source| Error::InvalidFollowsPath {
            path: input_val.clone(),
            source,
        })?;
        let target_path =
            AttrPath::parse(&target_val).map_err(|source| Error::InvalidFollowsPath {
                path: target_val.clone(),
                source,
            })?;
        // Reject paths deeper than 2 segments so a typo can't sneak through.
        // [`auto::run`] bypasses this guard via direct `apply_change` calls
        // and is gated by `config.follow.max_depth` instead.
        let segments = input_id.path().len();
        if segments > 2 {
            return Err(Error::Flake(FlakeError::AddFollowDepthLimit {
                path: input_id.to_string(),
                segments,
            }));
        }
        Change::Follows {
            input: input_id,
            target: target_path,
        }
    } else if state.interactive {
        let Some(ctx) = load_follow_context(flake_edit, state)? else {
            return Ok(());
        };
        let top_level_vec: Vec<String> = ctx.top_level_inputs.into_iter().collect();

        let tui_app = if let Some(input_val) = input {
            tui::App::follow_target("Follow", editor.text(), input_val, top_level_vec)
        } else {
            tui::App::follow("Follow", editor.text(), ctx.nested_inputs, top_level_vec)
        };

        let Some(tui::AppResult::Change(tui_change)) = tui::run(tui_app)? else {
            return Ok(());
        };
        tui_change
    } else {
        return Err(Error::NoId);
    };

    apply_change(editor, flake_edit, state, change)
}
