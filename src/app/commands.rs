use std::collections::HashSet;
use std::path::PathBuf;

use nix_uri::urls::UrlWrapper;
use nix_uri::{FlakeRef, NixUriResult};
use ropey::Rope;

use crate::change::{Change, ChangeId};
use crate::edit::{FlakeEdit, InputMap, sorted_input_ids, sorted_input_ids_owned};
use crate::error::FlakeEditError;
use crate::lock::{FlakeLock, NestedInput};
use crate::tui;
use crate::update::Updater;
use crate::validate;

use super::editor::Editor;
use super::state::AppState;

fn updater(editor: &Editor, inputs: InputMap) -> Updater {
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

pub(super) struct FollowContext {
    pub(super) nested_inputs: Vec<NestedInput>,
    pub(super) top_level_inputs: HashSet<String>,
    /// Full input map. Cycle detection needs URLs.
    pub(super) inputs: crate::edit::InputMap,
}

/// Load nested inputs from lockfile and top-level inputs from flake.nix.
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
            return Err(CommandError::LockFileError {
                path: lock_path,
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
fn interactive_single_select<F, OnApplied, ExtraData>(
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
fn interactive_multi_select<F>(
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

/// Apply `ref_or_rev` and `shallow` options to a [`FlakeRef`].
fn apply_uri_options(
    mut flake_ref: FlakeRef,
    ref_or_rev: Option<&str>,
    shallow: bool,
) -> std::result::Result<FlakeRef, String> {
    if let Some(ror) = ref_or_rev {
        flake_ref.r#type.ref_or_rev(Some(ror.to_string())).map_err(|e| {
            format!(
                "Cannot apply --ref-or-rev: {}. \
                The --ref-or-rev option only works with git forge types (github:, gitlab:, sourcehut:) and indirect types (flake:). \
                For other URI types, use ?ref= or ?rev= query parameters in the URI itself.",
                e
            )
        })?;
    }
    if shallow {
        flake_ref.params.set_shallow(Some("1".to_string()));
    }
    Ok(flake_ref)
}

/// Apply `ref_or_rev` and `shallow` to a URI string.
///
/// Always validates the URI through `nix-uri` parsing. If neither option
/// is set, returns the original URI unchanged.
fn transform_uri(uri: String, ref_or_rev: Option<&str>, shallow: bool) -> Result<String> {
    let flake_ref: FlakeRef = uri
        .parse()
        .map_err(|e| CommandError::InvalidUri(format!("{}: {}", uri, e)))?;

    if ref_or_rev.is_none() && !shallow {
        return Ok(uri);
    }

    apply_uri_options(flake_ref, ref_or_rev, shallow)
        .map(|f| f.to_string())
        .map_err(CommandError::CouldNotInferId)
}

#[derive(Default)]
pub struct UriOptions<'a> {
    pub ref_or_rev: Option<&'a str>,
    pub shallow: bool,
    pub no_flake: bool,
}

pub fn add(
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
    id: Option<String>,
    uri: Option<String>,
    opts: UriOptions<'_>,
) -> Result<()> {
    let change = match (id, uri, state.interactive) {
        // Both ID and URI provided: non-interactive add.
        (Some(id_val), Some(uri_str), _) => add_with_id_and_uri(id_val, uri_str, &opts)?,
        // Interactive: show TUI (with or without prefill).
        (id, None, true) | (None, id, true) => {
            add_interactive(editor, state, id.as_deref(), &opts)?
        }
        // Non-interactive with only one positional arg: infer ID from URI.
        (Some(uri), None, false) | (None, Some(uri), false) => add_infer_id(uri, &opts)?,
        (None, None, false) => {
            return Err(CommandError::NoUri);
        }
    };

    apply_change(editor, flake_edit, state, change)
}

fn add_with_id_and_uri(id: String, uri: String, opts: &UriOptions<'_>) -> Result<Change> {
    let final_uri = transform_uri(uri, opts.ref_or_rev, opts.shallow)?;
    Ok(Change::Add {
        id: Some(id),
        uri: Some(final_uri),
        flake: !opts.no_flake,
    })
}

fn add_interactive(
    editor: &Editor,
    state: &AppState,
    prefill_uri: Option<&str>,
    opts: &UriOptions<'_>,
) -> Result<Change> {
    let tui_app = tui::App::add("Add", editor.text(), prefill_uri, state.cache_config());
    let Some(tui::AppResult::Change(tui_change)) = tui::run(tui_app)? else {
        // User cancelled.
        return Ok(Change::None);
    };

    // CLI options override the TUI result.
    if let Change::Add { id, uri, flake } = tui_change {
        let final_uri = uri
            .map(|u| transform_uri(u, opts.ref_or_rev, opts.shallow))
            .transpose()?;
        Ok(Change::Add {
            id,
            uri: final_uri,
            flake: flake && !opts.no_flake,
        })
    } else {
        Ok(tui_change)
    }
}

/// Add with only URI: infer ID from the parsed flake reference.
fn add_infer_id(uri: String, opts: &UriOptions<'_>) -> Result<Change> {
    let flake_ref: NixUriResult<FlakeRef> = UrlWrapper::convert_or_parse(&uri);

    let (inferred_id, final_uri) = if let Ok(flake_ref) = flake_ref {
        let flake_ref = apply_uri_options(flake_ref, opts.ref_or_rev, opts.shallow)
            .map_err(CommandError::CouldNotInferId)?;
        let parsed_uri = flake_ref.to_string();
        let final_uri = if parsed_uri.is_empty() || parsed_uri == "none" {
            uri.clone()
        } else {
            parsed_uri
        };
        (flake_ref.id(), final_uri)
    } else {
        (None, uri.clone())
    };

    let final_id = inferred_id.ok_or(CommandError::CouldNotInferId(uri))?;

    Ok(Change::Add {
        id: Some(final_id),
        uri: Some(final_uri),
        flake: !opts.no_flake,
    })
}

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

pub fn change(
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
    id: Option<String>,
    uri: Option<String>,
    ref_or_rev: Option<&str>,
    shallow: bool,
) -> Result<()> {
    let inputs = flake_edit.list();

    let change = match (id, uri, state.interactive) {
        // Full interactive: select input, then enter URI. Also covers the
        // case where only URI was provided interactively (need to select input).
        (None, None, true) | (None, Some(_), true) => {
            change_full_interactive(editor, state, inputs, ref_or_rev, shallow)?
        }
        // ID provided, no URI, interactive: show URI input for that ID.
        (Some(id), None, true) => {
            change_uri_interactive(editor, state, inputs, &id, ref_or_rev, shallow)?
        }
        // Both ID and URI provided: non-interactive.
        (Some(id_val), Some(uri_str), _) => {
            change_with_id_and_uri(id_val, uri_str, ref_or_rev, shallow)?
        }
        // Only one positional arg: infer ID from URI.
        (Some(uri), None, false) | (None, Some(uri), false) => {
            change_infer_id(uri, ref_or_rev, shallow)?
        }
        (None, None, false) => {
            return Err(CommandError::NoId);
        }
    };

    apply_change(editor, flake_edit, state, change)
}

/// Interactive change: pick an input from the list, then enter the new URI.
fn change_full_interactive(
    editor: &Editor,
    state: &AppState,
    inputs: &crate::edit::InputMap,
    ref_or_rev: Option<&str>,
    shallow: bool,
) -> Result<Change> {
    let input_pairs: Vec<(String, String)> = sorted_input_ids(inputs)
        .into_iter()
        .map(|id| (id.clone(), inputs[id].url().trim_matches('"').to_string()))
        .collect();

    if input_pairs.is_empty() {
        return Err(CommandError::NoInputs);
    }

    let tui_app = tui::App::change("Change", editor.text(), input_pairs, state.cache_config());
    let Some(tui::AppResult::Change(tui_change)) = tui::run(tui_app)? else {
        return Ok(Change::None);
    };

    // CLI options override the TUI result.
    if let Change::Change { id, uri, .. } = tui_change {
        let final_uri = uri
            .map(|u| transform_uri(u, ref_or_rev, shallow))
            .transpose()?;
        Ok(Change::Change {
            id,
            uri: final_uri,
            ref_or_rev: None,
        })
    } else {
        Ok(tui_change)
    }
}

/// Interactive change with the ID already known: show only the URI input.
fn change_uri_interactive(
    editor: &Editor,
    state: &AppState,
    inputs: &crate::edit::InputMap,
    id: &str,
    ref_or_rev: Option<&str>,
    shallow: bool,
) -> Result<Change> {
    let current_uri = inputs.get(id).map(|i| i.url().trim_matches('"'));
    let tui_app = tui::App::change_uri(
        "Change",
        editor.text(),
        id,
        current_uri,
        state.diff,
        state.cache_config(),
    );

    let Some(tui::AppResult::Change(tui_change)) = tui::run(tui_app)? else {
        return Ok(Change::None);
    };

    // CLI options override the TUI result.
    if let Change::Change {
        uri: Some(new_uri), ..
    } = tui_change
    {
        let final_uri = transform_uri(new_uri, ref_or_rev, shallow)?;
        Ok(Change::Change {
            id: Some(id.to_string()),
            uri: Some(final_uri),
            ref_or_rev: None,
        })
    } else {
        Err(CommandError::NoUri)
    }
}

fn change_with_id_and_uri(
    id: String,
    uri: String,
    ref_or_rev: Option<&str>,
    shallow: bool,
) -> Result<Change> {
    let final_uri = transform_uri(uri, ref_or_rev, shallow)?;
    Ok(Change::Change {
        id: Some(id),
        uri: Some(final_uri),
        ref_or_rev: None,
    })
}

/// Change with only URI: infer ID from the parsed flake reference.
fn change_infer_id(uri: String, ref_or_rev: Option<&str>, shallow: bool) -> Result<Change> {
    let flake_ref: NixUriResult<FlakeRef> = UrlWrapper::convert_or_parse(&uri);

    let flake_ref = flake_ref.map_err(|_| CommandError::CouldNotInferId(uri.clone()))?;
    let flake_ref =
        apply_uri_options(flake_ref, ref_or_rev, shallow).map_err(CommandError::CouldNotInferId)?;

    let final_uri = if flake_ref.to_string().is_empty() {
        uri.clone()
    } else {
        flake_ref.to_string()
    };

    let id = flake_ref.id().ok_or(CommandError::CouldNotInferId(uri))?;

    Ok(Change::Change {
        id: Some(id),
        uri: Some(final_uri),
        ref_or_rev: None,
    })
}

pub fn update(
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
    id: Option<String>,
    init: bool,
) -> Result<()> {
    let inputs = flake_edit.list().clone();
    let input_ids = sorted_input_ids_owned(&inputs);

    if let Some(id) = id {
        let mut updater = updater(editor, inputs);
        updater.update_all_inputs_to_latest_semver(Some(id), init);
        let change = updater.get_changes();
        editor.apply_or_diff(&change, state)?;
    } else if state.interactive {
        if input_ids.is_empty() {
            return Err(CommandError::NoInputs);
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

pub fn pin(
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
    id: Option<String>,
    rev: Option<String>,
) -> Result<()> {
    let inputs = flake_edit.list().clone();
    let input_ids = sorted_input_ids_owned(&inputs);

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
    let input_ids = sorted_input_ids_owned(&inputs);

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

pub fn list(flake_edit: &mut FlakeEdit, format: &crate::cli::ListFormat) -> Result<()> {
    let inputs = flake_edit.list();
    crate::app::handler::list_inputs(inputs, format);
    Ok(())
}

/// Handler for the `config` subcommand.
pub fn config(print_default: bool, path: bool) -> Result<()> {
    use crate::config::{Config, DEFAULT_CONFIG_TOML};

    if print_default {
        print!("{}", DEFAULT_CONFIG_TOML);
        return Ok(());
    }

    if path {
        let project_path = Config::project_config_path();
        let user_path = Config::user_config_path();

        if let Some(path) = &project_path {
            println!("Project config: {}", path.display());
        }
        if let Some(path) = &user_path {
            println!("User config: {}", path.display());
        }

        if project_path.is_none() && user_path.is_none() {
            if let Some(user_dir) = Config::user_config_dir() {
                println!("No config found. Create one at:");
                println!("  Project: flake-edit.toml (in current directory)");
                println!("  User:    {}/config.toml", user_dir.display());
            } else {
                println!("No config found. Create flake-edit.toml in current directory.");
            }
        }
        return Ok(());
    }

    Ok(())
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
        assert!(sources.contains(&AttrPath::parse("home-manager.nixpkgs").unwrap()));
    }
}
