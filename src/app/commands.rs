use std::collections::{HashMap, HashSet};

use nix_uri::urls::UrlWrapper;
use nix_uri::{FlakeRef, NixUriResult};
use ropey::Rope;

use crate::change::Change;
use crate::edit::{FlakeEdit, InputMap, sorted_input_ids, sorted_input_ids_owned};
use crate::error::FlakeEditError;
use crate::input::Follows;
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

    #[error("The input could not be removed: {0}")]
    CouldNotRemove(String),
}

/// Load the flake.lock file, using the path from state if provided.
fn load_flake_lock(state: &AppState) -> std::result::Result<FlakeLock, FlakeEditError> {
    if let Some(lock_path) = &state.lock_file {
        FlakeLock::from_file(lock_path)
    } else {
        FlakeLock::from_default_path()
    }
}

struct FollowContext {
    nested_inputs: Vec<NestedInput>,
    top_level_inputs: HashSet<String>,
    /// Full input map for checking URLs (needed for cycle detection)
    inputs: crate::edit::InputMap,
}

/// Check if a top-level input's URL is a follows reference to a specific parent input.
/// For example, `treefmt-nix.follows = "clan-core/treefmt-nix"` has URL `"clan-core/treefmt-nix"`
/// which is a follows reference to the `clan-core` parent.
fn is_follows_reference_to_parent(url: &str, parent: &str) -> bool {
    let url_trimmed = url.trim_matches('"');
    url_trimmed.starts_with(&format!("{}/", parent))
}

/// Collect nested follows paths already declared in flake.nix.
fn collect_existing_follows(inputs: &InputMap) -> HashSet<String> {
    let mut existing = HashSet::new();
    for (input_id, input) in inputs {
        for follows in input.follows() {
            if let Follows::Indirect(nested_name, _target) = follows {
                existing.insert(format!("{}.{}", input_id, nested_name));
            }
        }
    }
    existing
}

/// Convert a lockfile follows path like "parent.child" to flake follows syntax "parent/child".
fn lock_follows_to_flake_target(target: &str) -> String {
    if target.contains('.') {
        target.replace('.', "/")
    } else {
        target.to_string()
    }
}

/// Load nested inputs from lockfile and top-level inputs from flake.nix.
fn load_follow_context(
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

    if top_level_inputs.is_empty() {
        return Err(CommandError::NoInputs);
    }

    Ok(Some(FollowContext {
        nested_inputs,
        top_level_inputs,
        inputs,
    }))
}

/// Result of running the confirm-or-apply workflow.
enum ConfirmResult {
    /// Change was applied successfully.
    Applied,
    /// User cancelled (Escape or window closed).
    Cancelled,
    /// User wants to go back to selection.
    Back,
}

/// Run an interactive single-select loop with confirmation.
///
/// 1. Show selection screen
/// 2. User selects an item
/// 3. Create change based on selection
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

/// Like `interactive_single_select` but for multi-selection.
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
///
/// Returns `Back` if user wants to go back to selection, `Applied` if the
/// change was applied, or `Cancelled` if the user cancelled.
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

/// Apply URI options (ref_or_rev, shallow) to a FlakeRef.
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

/// Transform a URI string by applying ref_or_rev and shallow options if specified.
///
/// Always validates the URI through nix-uri parsing.
/// If neither option is set, returns the original URI unchanged after validation.
/// Otherwise, applies the options and returns the transformed string.
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
        // Both ID and URI provided - non-interactive add
        (Some(id_val), Some(uri_str), _) => add_with_id_and_uri(id_val, uri_str, &opts)?,
        // Interactive mode - show TUI (with or without prefill)
        (id, None, true) | (None, id, true) => {
            add_interactive(editor, state, id.as_deref(), &opts)?
        }
        // Non-interactive with only one arg (could be in id or uri position) - infer ID
        (Some(uri), None, false) | (None, Some(uri), false) => add_infer_id(uri, &opts)?,
        // No arguments and non-interactive
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
        // User cancelled - return a no-op change
        return Ok(Change::None);
    };

    // Apply CLI options to the TUI result
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

/// Add with only URI provided, inferring ID from the flake reference.
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
    let change = if let Some(id) = id {
        Change::Remove {
            ids: vec![id.into()],
        }
    } else if state.interactive {
        let inputs = flake_edit.list();
        let mut removable: Vec<String> = Vec::new();
        for input_id in sorted_input_ids(inputs) {
            let input = &inputs[input_id];
            removable.push(input_id.clone());
            for follows in input.follows() {
                if let crate::input::Follows::Indirect(from, to) = follows {
                    removable.push(format!("{}.{} => {}", input_id, from, to));
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

        // Strip " => target" suffix for follows entries
        if let Change::Remove { ids } = tui_change {
            let stripped_ids: Vec<_> = ids
                .iter()
                .map(|id| {
                    id.to_string()
                        .split(" => ")
                        .next()
                        .unwrap_or(&id.to_string())
                        .to_string()
                        .into()
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
        // Full interactive: select input, then enter URI
        // Also handles case where only URI provided interactively (need to select input)
        (None, None, true) | (None, Some(_), true) => {
            change_full_interactive(editor, state, inputs, ref_or_rev, shallow)?
        }
        // ID provided, no URI, interactive: show URI input for that ID
        (Some(id), None, true) => {
            change_uri_interactive(editor, state, inputs, &id, ref_or_rev, shallow)?
        }
        // Both ID and URI provided - non-interactive
        (Some(id_val), Some(uri_str), _) => {
            change_with_id_and_uri(id_val, uri_str, ref_or_rev, shallow)?
        }
        // Only one positional arg (in id position), infer ID from URI
        (Some(uri), None, false) | (None, Some(uri), false) => {
            change_infer_id(uri, ref_or_rev, shallow)?
        }
        // No arguments and non-interactive
        (None, None, false) => {
            return Err(CommandError::NoId);
        }
    };

    apply_change(editor, flake_edit, state, change)
}

/// Full interactive change: select input from list, then enter new URI.
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

    // Apply CLI options to the TUI result
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

/// Interactive change with ID already known: show URI input.
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

    // Apply CLI options to the TUI result
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

/// Change with only URI provided, inferring ID from the flake reference.
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
                // Strip version suffix from display strings to get IDs
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
        let lock = FlakeLock::from_default_path().map_err(|e| CommandError::LockFileError {
            path: "flake.lock".to_string(),
            source: e,
        })?;
        let target_rev = if let Some(rev) = rev {
            rev
        } else {
            lock.rev_for(&id)
                .map_err(|_| CommandError::InputNotFound(id.clone()))?
        };
        let mut updater = updater(editor, inputs);
        updater.pin_input_to_ref(&id, &target_rev);
        let change = updater.get_changes();
        editor.apply_or_diff(&change, state)?;
        if !state.diff {
            println!("Pinned input: {} to {}", id, target_rev);
        }
    } else if state.interactive {
        if input_ids.is_empty() {
            return Err(CommandError::NoInputs);
        }
        let lock = FlakeLock::from_default_path().map_err(|e| CommandError::LockFileError {
            path: "flake.lock".to_string(),
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
                updater.pin_input_to_ref(id, &target_rev);
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
        updater.unpin_input(&id);
        let change = updater.get_changes();
        editor.apply_or_diff(&change, state)?;
        if !state.diff {
            println!("Unpinned input: {}", id);
        }
    } else if state.interactive {
        if input_ids.is_empty() {
            return Err(CommandError::NoInputs);
        }

        interactive_single_select(
            editor,
            state,
            "Unpin",
            "Select input",
            input_ids,
            |id| {
                let mut updater = updater(editor, inputs.clone());
                updater.unpin_input(id);
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

/// Handle the `config` subcommand.
pub fn config(print_default: bool, path: bool) -> Result<()> {
    use crate::config::{Config, DEFAULT_CONFIG_TOML};

    if print_default {
        print!("{}", DEFAULT_CONFIG_TOML);
        return Ok(());
    }

    if path {
        // Show where config would be loaded from
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

/// Manually add a single follows declaration.
pub fn add_follow(
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
    input: Option<String>,
    target: Option<String>,
) -> Result<()> {
    let change = if let (Some(input_val), Some(target_val)) = (input.clone(), target) {
        // Both provided - non-interactive
        Change::Follows {
            input: input_val.into(),
            target: target_val,
        }
    } else if state.interactive {
        // Interactive mode
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
        return Err(CommandError::NoId);
    };

    apply_change(editor, flake_edit, state, change)
}

/// Collect follows declarations that reference nested inputs
/// no longer present in the lock file.
fn collect_stale_follows(
    inputs: &InputMap,
    existing_nested_paths: &HashSet<String>,
) -> Vec<String> {
    let mut stale = Vec::new();
    for (input_id, input) in inputs {
        for follows in input.follows() {
            if let Follows::Indirect(nested_name, _target) = follows {
                let nested_path = format!("{}.{}", input_id, nested_name);
                if !existing_nested_paths.contains(&nested_path) {
                    stale.push(nested_path);
                }
            }
        }
    }
    stale
}

/// Automatically follow inputs based on lockfile information.
///
/// For each nested input (e.g., "crane.nixpkgs"), if there's a matching
/// top-level input with the same name (e.g., "nixpkgs"), create a follows
/// relationship. Skips inputs that already have follows set.
/// Also removes stale follows declarations that reference nested inputs
/// no longer present in the lock file.
///
/// The config file controls behavior:
/// - `follow.ignore`: List of input names to skip
/// - `follow.aliases`: Map of canonical names to alternatives (e.g., nixpkgs = ["nixpkgs-lib"])
/// - `follow.transitive_min`: Minimum number of matching transitive follows before adding a
///   top-level follows input (set to 0 to disable)
pub fn follow_auto(editor: &Editor, flake_edit: &mut FlakeEdit, state: &AppState) -> Result<()> {
    follow_auto_impl(editor, flake_edit, state, false)
}

/// Internal implementation with quiet flag for batch processing.
fn follow_auto_impl(
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
    quiet: bool,
) -> Result<()> {
    let Some(ctx) = load_follow_context(flake_edit, state)? else {
        if !quiet {
            println!("Nothing to deduplicate.");
        }
        return Ok(());
    };

    let existing_nested_paths: HashSet<String> = load_flake_lock(state)
        .map(|l| l.nested_input_paths().into_iter().collect())
        .unwrap_or_default();

    let to_unfollow = collect_stale_follows(&ctx.inputs, &existing_nested_paths);

    let follow_config = &state.config.follow;
    let existing_follows = collect_existing_follows(&ctx.inputs);
    let transitive_min = follow_config.transitive_min();
    let mut seen_nested: HashSet<String> = HashSet::new();

    // Collect candidates: nested inputs that match a top-level input
    let mut to_follow: Vec<(String, String)> = ctx
        .nested_inputs
        .iter()
        .filter_map(|nested| {
            let nested_name = nested.path.split('.').next_back().unwrap_or(&nested.path);
            let parent = nested.path.split('.').next().unwrap_or(&nested.path);

            // Skip ignored inputs (supports both full path and simple name)
            if follow_config.is_ignored(&nested.path, nested_name) {
                tracing::debug!("Skipping {}: ignored by config", nested.path);
                return None;
            }

            // Skip if already configured in flake.nix
            if existing_follows.contains(&nested.path) {
                tracing::debug!("Skipping {}: already follows in flake.nix", nested.path);
                return None;
            }

            // Find matching top-level input (direct match or via alias)
            let matching_top_level = ctx
                .top_level_inputs
                .iter()
                .find(|top| follow_config.can_follow(nested_name, top));

            let target = matching_top_level?;

            // Skip if target already follows from parent (would create cycle)
            // e.g., treefmt-nix.follows = "clan-core/treefmt-nix" means we can't
            // add clan-core.inputs.treefmt-nix.follows = "treefmt-nix"
            if let Some(target_input) = ctx.inputs.get(target.as_str())
                && is_follows_reference_to_parent(target_input.url(), parent)
            {
                tracing::debug!(
                    "Skipping {} -> {}: would create cycle (target follows {}/...)",
                    nested.path,
                    target,
                    parent
                );
                return None;
            }

            Some((nested.path.clone(), target.clone()))
        })
        .collect();

    for (nested_path, _target) in &to_follow {
        seen_nested.insert(nested_path.clone());
    }

    let mut transitive_groups: HashMap<String, HashMap<String, Vec<String>>> = HashMap::new();

    if transitive_min > 0 {
        for nested in ctx.nested_inputs.iter() {
            let nested_name = nested.path.split('.').next_back().unwrap_or(&nested.path);
            let parent = nested.path.split('.').next().unwrap_or(&nested.path);

            if follow_config.is_ignored(&nested.path, nested_name) {
                continue;
            }

            if existing_follows.contains(&nested.path) || seen_nested.contains(&nested.path) {
                continue;
            }

            let matching_top_level = ctx
                .top_level_inputs
                .iter()
                .find(|top| follow_config.can_follow(nested_name, top));

            if matching_top_level.is_some() {
                continue;
            }

            let Some(transitive_target) = nested.follows.as_ref() else {
                continue;
            };

            // Only consider transitive follows (path with a parent segment).
            if !transitive_target.contains('.') {
                continue;
            }

            // Avoid self-follow situations.
            if transitive_target == nested_name {
                continue;
            }

            let top_level_name = follow_config
                .resolve_alias(nested_name)
                .unwrap_or(nested_name)
                .to_string();

            // Skip if a top-level input already exists with that name.
            if ctx.top_level_inputs.contains(&top_level_name) {
                continue;
            }

            // Skip if target already follows from parent (would create cycle)
            if let Some(target_input) = ctx.inputs.get(transitive_target.as_str())
                && is_follows_reference_to_parent(target_input.url(), parent)
            {
                continue;
            }

            transitive_groups
                .entry(top_level_name)
                .or_default()
                .entry(transitive_target.clone())
                .or_default()
                .push(nested.path.clone());
        }
    }

    // Pass 2b: Group Direct references (follows: None) by canonical name.
    // These are nested inputs that point to separate lock nodes rather than
    // following an existing path. When multiple parents share the same
    // dependency (e.g., treefmt.nixpkgs and treefmt-nix.nixpkgs), we can
    // promote one to top-level and have the others follow it.
    // Each entry: canonical_name -> Vec<(path, url)>
    let mut direct_groups: HashMap<String, Vec<(String, Option<String>)>> = HashMap::new();

    if transitive_min > 0 {
        for nested in ctx.nested_inputs.iter() {
            let nested_name = nested.path.split('.').next_back().unwrap_or(&nested.path);

            if nested.follows.is_some() {
                continue;
            }

            if follow_config.is_ignored(&nested.path, nested_name) {
                continue;
            }

            if existing_follows.contains(&nested.path) || seen_nested.contains(&nested.path) {
                continue;
            }

            let matching_top_level = ctx
                .top_level_inputs
                .iter()
                .find(|top| follow_config.can_follow(nested_name, top));

            if matching_top_level.is_some() {
                continue;
            }

            let canonical_name = follow_config
                .resolve_alias(nested_name)
                .unwrap_or(nested_name)
                .to_string();

            if ctx.top_level_inputs.contains(&canonical_name) {
                continue;
            }

            direct_groups
                .entry(canonical_name)
                .or_default()
                .push((nested.path.clone(), nested.url.clone()));
        }
    }

    let mut toplevel_follows: Vec<(String, String)> = Vec::new();
    let mut toplevel_adds: Vec<(String, String)> = Vec::new();

    if transitive_min > 0 {
        for (top_name, targets) in transitive_groups {
            let mut eligible: Vec<(String, Vec<String>)> = targets
                .into_iter()
                .filter(|(_, paths)| paths.len() >= transitive_min)
                .collect();

            if eligible.len() != 1 {
                continue;
            }

            let (target_path, paths) = eligible.pop().unwrap();
            let follow_target = lock_follows_to_flake_target(&target_path);

            if follow_target == top_name {
                continue;
            }

            toplevel_follows.push((top_name.clone(), follow_target));

            for path in paths {
                if seen_nested.insert(path.clone()) {
                    to_follow.push((path, top_name.clone()));
                }
            }
        }

        // Promote Direct reference groups: add a new top-level input with the
        // URL from one of the nested references, then have all paths follow it.
        // Only promote if at least one follows can actually be applied.
        let mut direct_groups_sorted: Vec<_> = direct_groups.into_iter().collect();
        direct_groups_sorted.sort_by(|a, b| a.0.cmp(&b.0));
        for (canonical_name, mut entries) in direct_groups_sorted {
            if entries.len() < transitive_min {
                continue;
            }

            entries.sort_by(|a, b| a.0.cmp(&b.0));

            let url = entries.iter().find_map(|(_, u)| u.clone());
            let Some(url) = url else {
                continue;
            };

            // Dry-run: check that at least one follows can be applied.
            let can_follow = entries.iter().any(|(path, _)| {
                let change = Change::Follows {
                    input: path.clone().into(),
                    target: canonical_name.clone(),
                };
                FlakeEdit::from_text(&editor.text())
                    .ok()
                    .and_then(|mut fe| fe.apply_change(change).ok().flatten())
                    .is_some()
            });
            if !can_follow {
                continue;
            }

            toplevel_adds.push((canonical_name.clone(), url));

            for (path, _) in &entries {
                if seen_nested.insert(path.clone()) {
                    to_follow.push((path.clone(), canonical_name.clone()));
                }
            }
        }
    }

    if to_follow.is_empty()
        && to_unfollow.is_empty()
        && toplevel_follows.is_empty()
        && toplevel_adds.is_empty()
    {
        if !quiet {
            println!("All inputs are already deduplicated.");
        }
        return Ok(());
    }

    // Apply all changes in memory
    let mut current_text = editor.text();
    let mut applied: Vec<(&str, &str)> = Vec::new();

    // First, add new top-level inputs (from Direct reference promotion).
    // These must be added before follows declarations that reference them.
    for (id, url) in &toplevel_adds {
        let change = Change::Add {
            id: Some(id.clone()),
            uri: Some(url.clone()),
            flake: true,
        };

        let mut temp_flake_edit =
            FlakeEdit::from_text(&current_text).map_err(CommandError::FlakeEdit)?;

        match temp_flake_edit.apply_change(change) {
            Ok(Some(resulting_text)) => {
                let validation = validate::validate(&resulting_text);
                if validation.is_ok() {
                    current_text = resulting_text;
                } else {
                    for err in validation.errors {
                        eprintln!("Error adding top-level input {}: {}", id, err);
                    }
                }
            }
            Ok(None) => eprintln!("Could not add top-level input {}", id),
            Err(e) => eprintln!("Error adding top-level input {}: {}", id, e),
        }
    }

    let mut follow_changes: Vec<(String, String)> = Vec::new();
    follow_changes.extend(toplevel_follows.into_iter());
    follow_changes.extend(to_follow.into_iter());

    for (input_path, target) in &follow_changes {
        let change = Change::Follows {
            input: input_path.clone().into(),
            target: target.clone(),
        };

        let mut temp_flake_edit =
            FlakeEdit::from_text(&current_text).map_err(CommandError::FlakeEdit)?;

        match temp_flake_edit.apply_change(change) {
            Ok(Some(resulting_text)) => {
                let validation = validate::validate(&resulting_text);
                if validation.is_ok() {
                    current_text = resulting_text;
                    applied.push((input_path, target));
                } else {
                    for err in validation.errors {
                        eprintln!("Error applying follows for {}: {}", input_path, err);
                    }
                }
            }
            Ok(None) => eprintln!("Could not create follows for {}", input_path),
            Err(e) => eprintln!("Error applying follows for {}: {}", input_path, e),
        }
    }

    let mut unfollowed: Vec<&str> = Vec::new();

    for nested_path in &to_unfollow {
        let change = Change::Remove {
            ids: vec![nested_path.clone().into()],
        };

        let mut temp_flake_edit =
            FlakeEdit::from_text(&current_text).map_err(CommandError::FlakeEdit)?;

        match temp_flake_edit.apply_change(change) {
            Ok(Some(resulting_text)) => {
                let validation = validate::validate(&resulting_text);
                if validation.is_ok() {
                    current_text = resulting_text;
                    unfollowed.push(nested_path);
                }
            }
            Ok(None) => {}
            Err(e) => eprintln!("Error removing stale follows for {}: {}", nested_path, e),
        }
    }

    if applied.is_empty() && unfollowed.is_empty() {
        return Ok(());
    }

    if state.diff {
        let original = editor.text();
        let diff = crate::diff::Diff::new(&original, &current_text);
        diff.compare();
    } else {
        editor.apply_or_diff(&current_text, state)?;

        if !quiet {
            if !applied.is_empty() {
                println!(
                    "Deduplicated {} {}.",
                    applied.len(),
                    if applied.len() == 1 {
                        "input"
                    } else {
                        "inputs"
                    }
                );
                for (input_path, target) in &applied {
                    if let Some((parent, _)) = input_path.split_once('.') {
                        let nested_name = input_path.split('.').next_back().unwrap_or(input_path);
                        println!("  {}.{} → {}", parent, nested_name, target);
                    } else {
                        println!("  {} → {}", input_path, target);
                    }
                }
            }

            if !unfollowed.is_empty() {
                println!(
                    "Removed {} stale follows {}.",
                    unfollowed.len(),
                    if unfollowed.len() == 1 {
                        "declaration"
                    } else {
                        "declarations"
                    }
                );
                for path in &unfollowed {
                    println!("  {} (input no longer exists)", path);
                }
            }
        }
    }

    Ok(())
}

/// Process multiple flake files in batch mode.
///
/// Each file is processed independently with its own Editor/AppState.
/// Errors are collected and reported at the end, but processing continues
/// for all files. Returns error if any file failed.
pub fn follow_auto_batch(paths: &[std::path::PathBuf], args: &crate::cli::CliArgs) -> Result<()> {
    use std::path::PathBuf;

    let mut errors: Vec<(PathBuf, CommandError)> = Vec::new();

    for flake_path in paths {
        let lock_path = flake_path
            .parent()
            .map(|p| p.join("flake.lock"))
            .unwrap_or_else(|| PathBuf::from("flake.lock"));

        let editor = match Editor::from_path(flake_path.clone()) {
            Ok(e) => e,
            Err(e) => {
                errors.push((flake_path.clone(), e.into()));
                continue;
            }
        };

        let mut flake_edit = match editor.create_flake_edit() {
            Ok(fe) => fe,
            Err(e) => {
                errors.push((flake_path.clone(), e.into()));
                continue;
            }
        };

        let state = match AppState::new(
            editor.text(),
            flake_path.clone(),
            args.config().map(PathBuf::from),
        ) {
            Ok(s) => s
                .with_diff(args.diff())
                .with_no_lock(args.no_lock())
                .with_interactive(false)
                .with_lock_file(Some(lock_path))
                .with_no_cache(args.no_cache())
                .with_cache_path(args.cache().map(PathBuf::from)),
            Err(e) => {
                errors.push((flake_path.clone(), e.into()));
                continue;
            }
        };

        if let Err(e) = follow_auto_impl(&editor, &mut flake_edit, &state, true) {
            errors.push((flake_path.clone(), e));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        for (path, err) in &errors {
            eprintln!("Error processing {}: {}", path.display(), err);
        }
        // Return the first error
        Err(errors.into_iter().next().unwrap().1)
    }
}

fn apply_change(
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
    change: Change,
) -> Result<()> {
    match flake_edit.apply_change(change.clone()) {
        Ok(Some(resulting_change)) => {
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
                // Cache added entries for future completions
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
        }
        Err(e) => {
            return Err(e.into());
        }
        Ok(None) => {
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
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_follows_reference_to_parent() {
        // Test case: treefmt-nix.follows = "clan-core/treefmt-nix"
        // The URL would be stored as "\"clan-core/treefmt-nix\""
        assert!(is_follows_reference_to_parent(
            "\"clan-core/treefmt-nix\"",
            "clan-core"
        ));

        // Also test without surrounding quotes (defensive)
        assert!(is_follows_reference_to_parent(
            "clan-core/treefmt-nix",
            "clan-core"
        ));

        // Test with different parent
        assert!(is_follows_reference_to_parent(
            "\"some-input/nixpkgs\"",
            "some-input"
        ));

        // Negative test: regular URL should not match
        assert!(!is_follows_reference_to_parent(
            "\"github:nixos/nixpkgs\"",
            "clan-core"
        ));

        // Negative test: URL that contains the parent but doesn't start with it
        assert!(!is_follows_reference_to_parent(
            "\"github:foo/clan-core-utils\"",
            "clan-core"
        ));

        // Negative test: parent name matches but not followed by /
        assert!(!is_follows_reference_to_parent(
            "\"clan-core-extended\"",
            "clan-core"
        ));

        // Edge case: empty URL
        assert!(!is_follows_reference_to_parent("", "clan-core"));

        // Edge case: just quotes
        assert!(!is_follows_reference_to_parent("\"\"", "clan-core"));
    }
}
