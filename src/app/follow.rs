//! Implementations of `flake-edit follow` and `flake-edit add-follow`.
//!
//! - [`add_follow`] is the single-shot explicit-path command, fronting both
//!   the scripted `<input> <target>` form and the interactive TUI flow.
//! - [`auto`] is the auto-deduplication subsystem: planner, applier, and
//!   batch driver for `flake-edit follow [PATHS...]`.
//!
//! Both share `FollowContext` plumbing in [`super::commands`].

pub mod auto;

use crate::change::{Change, ChangeId};
use crate::edit::FlakeEdit;
use crate::error::FlakeEditError;
use crate::follows::AttrPath;
use crate::tui;

use super::commands::{self, CommandError, Result};
use super::editor::Editor;
use super::state::AppState;

/// Manually add a single follows declaration.
pub fn add_follow(
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
    input: Option<String>,
    target: Option<String>,
) -> Result<()> {
    let change = if let (Some(input_val), Some(target_val)) = (input.clone(), target) {
        let input_id = ChangeId::parse(&input_val).map_err(|e| {
            CommandError::InvalidUri(format!("invalid follows path `{input_val}`: {e}"))
        })?;
        let target_path = AttrPath::parse(&target_val).map_err(|e| {
            CommandError::InvalidUri(format!("invalid follows target `{target_val}`: {e}"))
        })?;
        // Reject paths deeper than 2 segments so a typo can't sneak through.
        // [`auto::run`] bypasses this guard via direct `apply_change` calls
        // and is gated by `config.follow.max_depth` instead.
        let segments = input_id.path().len();
        if segments > 2 {
            return Err(CommandError::FlakeEdit(
                FlakeEditError::AddFollowDepthLimit {
                    path: input_id.to_string(),
                    segments,
                },
            ));
        }
        Change::Follows {
            input: input_id,
            target: target_path,
        }
    } else if state.interactive {
        let Some(ctx) = commands::load_follow_context(flake_edit, state)? else {
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
        return Err(CommandError::NoId);
    };

    commands::apply_change(editor, flake_edit, state, change)
}
