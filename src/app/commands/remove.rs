//! `flake-edit remove`: drop an input or follows entry from the flake.
//!
//! Scripted mode takes a [`ChangeId`] directly. Interactive mode
//! shows a picker over both top-level inputs and their indirect
//! follows; follows entries display as `parent.nested => target` so
//! the user sees the disconnected target, and the suffix is stripped
//! before parsing back to a [`ChangeId`].

use crate::change::{Change, ChangeId};
use crate::edit::{FlakeEdit, sorted_input_ids};
use crate::tui;

use super::super::editor::Editor;
use super::super::state::AppState;
use super::{CommandError, Result, apply_change};

pub fn remove(
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
    id: Option<String>,
) -> Result<()> {
    let change =
        if let Some(id) = id {
            Change::Remove {
                ids: vec![ChangeId::parse(&id).map_err(|e| {
                    CommandError::InvalidUri(format!("invalid input id `{id}`: {e}"))
                })?],
            }
        } else if state.interactive {
            let inputs = flake_edit.list();
            let mut removable: Vec<String> = Vec::new();
            for input_id in sorted_input_ids(inputs) {
                let input = &inputs[input_id];
                removable.push(input_id.clone());
                for follows in input.follows() {
                    if let crate::input::Follows::Indirect { path, target } = follows {
                        let target_str = match target {
                            Some(t) => t.to_string(),
                            None => "\"\"".to_string(),
                        };
                        removable.push(format!("{}.{} => {}", input_id, path, target_str));
                    }
                }
            }
            if removable.is_empty() {
                return Err(CommandError::NoInputs);
            }

            let tui_app = tui::App::remove("Remove", editor.text(), removable);
            let Some(tui::AppResult::Change(tui_change)) = tui::run(tui_app)? else {
                return Ok(());
            };

            // Strip the " => target" suffix on follows entries.
            if let Change::Remove { ids } = tui_change {
                let stripped_ids: Vec<_> = ids
                    .iter()
                    .filter_map(|id| {
                        let s = id.to_string();
                        let stripped = s.split(" => ").next().unwrap_or(&s);
                        ChangeId::parse(stripped).ok()
                    })
                    .collect();
                Change::Remove { ids: stripped_ids }
            } else {
                tui_change
            }
        } else {
            return Err(CommandError::NoId);
        };

    apply_change(editor, flake_edit, state, change)
}
