use std::path::PathBuf;

use ropey::Rope;

use crate::change::Change;
use crate::edit::{FlakeEdit, InputMap};
use crate::error::FlakeEditError;
use crate::forge::update::Updater;
use crate::lock::FlakeLock;
use crate::tui;
use crate::validate;

use super::editor::Editor;
use super::state::AppState;

mod add;
mod change;
mod config;
pub mod follow;
pub mod list;
mod pin;
mod remove;
mod update;
mod uri;

pub use add::add;
pub use change::change;
pub use config::config;
pub use list::list;
pub use pin::{pin, unpin};
pub use remove::remove;
pub use update::update;
pub use uri::UriOptions;

pub(super) fn updater(editor: &Editor, inputs: InputMap) -> Updater {
    Updater::new(Rope::from_str(&editor.text()), inputs)
}

pub type Result<T> = std::result::Result<T, CommandError>;

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error(transparent)]
    FlakeEdit(#[from] FlakeEditError),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Config(#[from] crate::config::ConfigError),

    #[error("No URI provided")]
    NoUri,

    #[error("No ID provided")]
    NoId,

    #[error("Could not infer ID from flake reference: {0}")]
    CouldNotInferId(String),

    #[error("Invalid URI: {0}")]
    InvalidUri(String),

    #[error("No inputs found in the flake")]
    NoInputs,

    #[error("Could not read lock file '{path}': {source}")]
    LockFileError {
        path: String,
        source: FlakeEditError,
    },

    #[error("Input not found: {0}")]
    InputNotFound(String),

    #[error("Input '{0}' has no pinnable URL (it may use follows or a non-standard format)")]
    InputNotPinnable(String),

    #[error("The input could not be removed: {0}")]
    CouldNotRemove(String),

    /// Aggregated failures from a `follow [PATHS...]` batch. Each entry
    /// pairs the offending path with the error processing it produced.
    #[error("{} file(s) failed during batch processing:\n{}", failures.len(), format_batch_failures(failures))]
    Batch {
        failures: Vec<(PathBuf, CommandError)>,
    },
}

fn format_batch_failures(failures: &[(PathBuf, CommandError)]) -> String {
    failures
        .iter()
        .map(|(path, err)| format!("  - {}: {}", path.display(), err))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Load `flake.lock`, using the path from `state` if provided.
pub(super) fn load_flake_lock(state: &AppState) -> std::result::Result<FlakeLock, FlakeEditError> {
    if let Some(lock_path) = &state.lock_file {
        FlakeLock::from_file(lock_path)
    } else {
        FlakeLock::from_default_path()
    }
}

/// Outcome of [`confirm_or_apply`].
enum ConfirmResult {
    /// Change was applied successfully.
    Applied,
    /// User cancelled (Escape or window closed).
    Cancelled,
    /// User wants to go back to selection.
    Back,
}

/// Interactive single-select loop with confirmation.
///
/// 1. Show selection screen
/// 2. User selects an item
/// 3. Build a change from the selection
/// 4. Show confirmation (with diff if requested)
/// 5. Apply or go back
pub(super) fn interactive_single_select<F, OnApplied, ExtraData>(
    editor: &Editor,
    state: &AppState,
    title: &str,
    prompt: &str,
    items: Vec<String>,
    make_change: F,
    on_applied: OnApplied,
) -> Result<()>
where
    F: Fn(&str) -> Result<(String, ExtraData)>,
    OnApplied: Fn(&str, ExtraData),
{
    loop {
        let select_app = tui::App::select_one(title, prompt, items.clone(), state.diff);
        let Some(tui::AppResult::SingleSelect(result)) = tui::run(select_app)? else {
            return Ok(());
        };
        let tui::SingleSelectResult {
            item: id,
            show_diff,
        } = result;
        let (change, extra_data) = make_change(&id)?;

        match confirm_or_apply(editor, state, title, &change, show_diff)? {
            ConfirmResult::Applied => {
                on_applied(&id, extra_data);
                break;
            }
            ConfirmResult::Back => continue,
            ConfirmResult::Cancelled => return Ok(()),
        }
    }
    Ok(())
}

/// Multi-select counterpart of [`interactive_single_select`].
pub(super) fn interactive_multi_select<F>(
    editor: &Editor,
    state: &AppState,
    title: &str,
    prompt: &str,
    items: Vec<String>,
    make_change: F,
) -> Result<()>
where
    F: Fn(&[String]) -> String,
{
    loop {
        let select_app = tui::App::select_many(title, prompt, items.clone(), state.diff);
        let Some(tui::AppResult::MultiSelect(result)) = tui::run(select_app)? else {
            return Ok(());
        };
        let tui::MultiSelectResultData {
            items: selected,
            show_diff,
        } = result;
        let change = make_change(&selected);

        match confirm_or_apply(editor, state, title, &change, show_diff)? {
            ConfirmResult::Applied => break,
            ConfirmResult::Back => continue,
            ConfirmResult::Cancelled => return Ok(()),
        }
    }
    Ok(())
}

/// Run the confirm-or-apply workflow for a change.
///
/// If `show_diff` is true, shows a confirmation screen with the diff.
/// Otherwise applies the change directly.
fn confirm_or_apply(
    editor: &Editor,
    state: &AppState,
    context: &str,
    change: &str,
    show_diff: bool,
) -> Result<ConfirmResult> {
    if show_diff || state.diff {
        let diff = crate::diff::Diff::new(&editor.text(), change).to_string_plain();
        let confirm_app = tui::App::confirm(context, &diff);
        let Some(tui::AppResult::Confirm(action)) = tui::run(confirm_app)? else {
            return Ok(ConfirmResult::Cancelled);
        };
        match action {
            tui::ConfirmResultAction::Apply => {
                let mut apply_state = state.clone();
                apply_state.diff = false;
                editor.apply_or_diff(change, &apply_state)?;
                Ok(ConfirmResult::Applied)
            }
            tui::ConfirmResultAction::Back => Ok(ConfirmResult::Back),
            tui::ConfirmResultAction::Exit => Ok(ConfirmResult::Cancelled),
        }
    } else {
        editor.apply_or_diff(change, state)?;
        Ok(ConfirmResult::Applied)
    }
}

pub(super) fn apply_change(
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
    change: Change,
) -> Result<()> {
    let original_content = flake_edit.source_text();
    let outcome = flake_edit.apply_change(change.clone())?;
    let resulting_change = match outcome.text {
        Some(t) => t,
        None => {
            if change.is_remove() {
                return Err(CommandError::CouldNotRemove(
                    change.id().map(|id| id.to_string()).unwrap_or_default(),
                ));
            }
            if change.is_follows() {
                let id = change.id().map(|id| id.to_string()).unwrap_or_default();
                eprintln!("The follows relationship for {} could not be created.", id);
                eprintln!(
                    "\nPlease check that the input exists in the flake.nix file.\n\
                     Use dot notation: `flake-edit follow <input>.<nested-input> <target>`\n\
                     Example: `flake-edit follow rust-overlay.nixpkgs nixpkgs`"
                );
                std::process::exit(1);
            }
            println!("Nothing changed.");
            return Ok(());
        }
    };

    if change.is_follows() && resulting_change == original_content {
        if let Some(id) = change.id() {
            let follows_str = id
                .follows()
                .map(|s| s.render())
                .unwrap_or_else(|| "?".to_string());
            let target_str = change
                .follows_target()
                .map(|t| t.to_string())
                .unwrap_or_else(|| "?".to_string());
            println!(
                "Already follows: {}.inputs.{}.follows = \"{}\"",
                id.input().render(),
                follows_str,
                target_str,
            );
        }
        return Ok(());
    }

    let validation = validate::validate(&resulting_change);
    if validation.has_errors() {
        eprintln!("There are errors in the changes:");
        for e in &validation.errors {
            tracing::error!("Error: {e}");
        }
        eprintln!("{}", resulting_change);
        eprintln!("There were errors in the changes, the changes have not been applied.");
        std::process::exit(1);
    }

    editor.apply_or_diff(&resulting_change, state)?;

    if !state.diff {
        // Cache added entries for future completions.
        if let Change::Add {
            id: Some(id),
            uri: Some(uri),
            ..
        } = &change
        {
            let mut cache = crate::cache::Cache::load();
            cache.add_entry(id.clone(), uri.clone());
            if let Err(e) = cache.commit() {
                tracing::debug!("Could not write to cache: {}", e);
            }
        }

        for msg in change.success_messages() {
            println!("{}", msg);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::follows::AttrPath;

    #[test]
    fn existing_follows_via_graph_handles_quoted_attrs() {
        use crate::follows::{FollowsGraph, Segment};
        use crate::input::{Follows, Input};

        // `"home-manager".inputs.nixpkgs.follows = "nixpkgs"` must resolve to
        // a typed-AttrPath edge sourced at `home-manager.nixpkgs`, not at the
        // quoted form.
        let mut inputs = InputMap::new();
        let hm_seg = Segment::from_unquoted("home-manager").unwrap();
        let mut hm_input = Input::new(hm_seg.clone());
        hm_input.follows.push(Follows::Indirect {
            path: AttrPath::new(Segment::from_unquoted("nixpkgs").unwrap()),
            target: Some(AttrPath::parse("nixpkgs").unwrap()),
        });
        inputs.insert("home-manager".to_string(), hm_input);

        let graph = FollowsGraph::from_declared(&inputs);
        let sources: HashSet<AttrPath> = graph.edges().map(|e| e.source.clone()).collect();
        assert!(
            sources.contains(&AttrPath::parse("home-manager.nixpkgs").unwrap()),
            "expected typed-AttrPath edge sourced at `home-manager.nixpkgs`, got {sources:?}",
        );
    }
}
