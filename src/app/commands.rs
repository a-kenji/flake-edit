use nix_uri::urls::UrlWrapper;
use nix_uri::{FlakeRef, NixUriResult};
use ropey::Rope;

use crate::change::Change;
use crate::edit::{FlakeEdit, InputMap, sorted_input_ids, sorted_input_ids_owned};
use crate::error::FlakeEditError;
use crate::lock::{FlakeLock, NestedInput};
use crate::tui;
use crate::update::Updater;

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

    #[error("No URI provided")]
    NoUri,

    #[error("No ID provided")]
    NoId,

    #[error("Could not infer ID from flake reference: {0}")]
    CouldNotInferId(String),

    #[error("No inputs found in the flake")]
    NoInputs,

    #[error("Could not read flake.lock")]
    NoLock,

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
    top_level_inputs: std::collections::HashSet<String>,
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

/// Load nested inputs from lockfile and top-level inputs from flake.nix.
/// Returns None if no nested inputs found (prints message to stderr).
fn load_follow_context(
    flake_edit: &mut FlakeEdit,
    state: &AppState,
) -> Result<Option<FollowContext>> {
    let nested_inputs: Vec<NestedInput> = load_flake_lock(state)
        .map(|lock| lock.get_nested_inputs())
        .unwrap_or_default();

    if nested_inputs.is_empty() {
        eprintln!("No nested inputs found in flake.lock");
        eprintln!("Make sure you have run `nix flake lock` first.");
        return Ok(None);
    }

    let inputs = flake_edit.list().clone();
    let top_level_inputs: std::collections::HashSet<String> = inputs.keys().cloned().collect();

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
/// If neither option is set, returns the original URI unchanged.
/// Otherwise, parses the URI, applies the options, and returns the transformed string.
fn transform_uri(uri: String, ref_or_rev: Option<&str>, shallow: bool) -> Result<String> {
    if ref_or_rev.is_none() && !shallow {
        return Ok(uri);
    }

    let flake_ref: FlakeRef = uri
        .parse()
        .map_err(|e| CommandError::CouldNotInferId(format!("{}: {}", uri, e)))?;

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
        let lock = FlakeLock::from_default_path().map_err(|_| CommandError::NoLock)?;
        let target_rev = if let Some(rev) = rev {
            rev
        } else {
            lock.get_rev_by_id(&id)
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
        let lock = FlakeLock::from_default_path().map_err(|_| CommandError::NoLock)?;

        interactive_single_select(
            editor,
            state,
            "Pin",
            "Select input",
            input_ids,
            |id| {
                let target_rev = lock
                    .get_rev_by_id(id)
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

pub fn follow(
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
    input: Option<String>,
    target: Option<String>,
    auto: bool,
) -> Result<()> {
    if auto {
        return follow_auto(editor, flake_edit, state);
    }

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

/// Automatically follow inputs based on lockfile information.
///
/// For each nested input (e.g., "crane.nixpkgs"), if there's a matching
/// top-level input with the same name (e.g., "nixpkgs"), create a follows
/// relationship. Skips inputs that already have follows set.
fn follow_auto(editor: &Editor, flake_edit: &mut FlakeEdit, state: &AppState) -> Result<()> {
    let Some(ctx) = load_follow_context(flake_edit, state)? else {
        return Ok(());
    };

    // Collect candidates: nested inputs that match a top-level input
    let to_follow: Vec<(String, String)> = ctx
        .nested_inputs
        .iter()
        .filter(|nested| nested.follows.is_none())
        .filter_map(|nested| {
            let nested_name = nested.path.split('.').next_back().unwrap_or(&nested.path);
            let parent = nested.path.split('.').next().unwrap_or(&nested.path);

            if !ctx.top_level_inputs.contains(nested_name) {
                return None;
            }

            // Skip if target already follows from parent (would create cycle)
            // e.g., treefmt-nix.follows = "clan-core/treefmt-nix" means we can't
            // add clan-core.inputs.treefmt-nix.follows = "treefmt-nix"
            if let Some(target_input) = ctx.inputs.get(nested_name)
                && is_follows_reference_to_parent(target_input.url(), parent)
            {
                tracing::debug!(
                    "Skipping {} -> {}: would create cycle (target follows {}/...)",
                    nested.path,
                    nested_name,
                    parent
                );
                return None;
            }

            Some((nested.path.clone(), nested_name.to_string()))
        })
        .collect();

    if to_follow.is_empty() {
        println!("All inputs are already deduplicated.");
        return Ok(());
    }

    // Apply all changes in memory
    let mut current_text = editor.text();
    let mut applied: Vec<(&str, &str)> = Vec::new();

    for (input_path, target) in &to_follow {
        let change = Change::Follows {
            input: input_path.clone().into(),
            target: target.clone(),
        };

        let mut temp_flake_edit =
            FlakeEdit::from_text(&current_text).map_err(CommandError::FlakeEdit)?;

        match temp_flake_edit.apply_change(change) {
            Ok(Some(resulting_text)) => {
                let root = rnix::Root::parse(&resulting_text);
                if root.errors().is_empty() {
                    current_text = resulting_text;
                    applied.push((input_path, target));
                } else {
                    eprintln!("Error applying follows for {}: parse errors", input_path);
                }
            }
            Ok(None) => eprintln!("Could not create follows for {}", input_path),
            Err(e) => eprintln!("Error applying follows for {}: {}", input_path, e),
        }
    }

    if applied.is_empty() {
        return Ok(());
    }

    if state.diff {
        let original = editor.text();
        let diff = crate::diff::Diff::new(&original, &current_text);
        diff.compare();
    } else {
        editor.apply_or_diff(&current_text, state)?;
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
            let nested_name = input_path.split('.').next_back().unwrap_or(input_path);
            let parent = input_path.split('.').next().unwrap_or(input_path);
            println!("  {}.{} → {}", parent, nested_name, target);
        }
    }

    Ok(())
}

fn apply_change(
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
    change: Change,
) -> Result<()> {
    match flake_edit.apply_change(change.clone()) {
        Ok(Some(resulting_change)) => {
            let root = rnix::Root::parse(&resulting_change);
            let errors = root.errors();
            if !errors.is_empty() {
                eprintln!("There are errors in the changes:");
                for e in errors {
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
