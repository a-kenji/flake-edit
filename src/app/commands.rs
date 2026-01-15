use nix_uri::urls::UrlWrapper;
use nix_uri::{FlakeRef, NixUriResult};
use ropey::Rope;

use crate::change::Change;
use crate::edit::FlakeEdit;
use crate::error::FlakeEditError;
use crate::lock::{FlakeLock, NestedInput};
use crate::tui;
use crate::update::Updater;

use super::editor::Editor;
use super::state::AppState;

pub type Result<T> = std::result::Result<T, CommandError>;

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("{0}")]
    FlakeEdit(#[from] FlakeEditError),

    #[error("{0}")]
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

    let inputs = flake_edit.list();
    let top_level_inputs: std::collections::HashSet<String> = inputs.keys().cloned().collect();

    if top_level_inputs.is_empty() {
        return Err(CommandError::NoInputs);
    }

    Ok(Some(FollowContext {
        nested_inputs,
        top_level_inputs,
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
pub fn apply_uri_options(
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

#[allow(clippy::too_many_arguments)]
pub fn add(
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
    id: Option<String>,
    uri: Option<String>,
    ref_or_rev: Option<&str>,
    no_flake: bool,
    shallow: bool,
) -> Result<()> {
    let change = if let (Some(id_val), Some(uri_str)) = (id.clone(), uri) {
        // Both provided - non-interactive
        let final_uri = if ref_or_rev.is_some() || shallow {
            let flake_ref: FlakeRef = uri_str
                .parse()
                .map_err(|e| CommandError::CouldNotInferId(format!("{}: {}", uri_str, e)))?;
            apply_uri_options(flake_ref, ref_or_rev, shallow)
                .map_err(CommandError::CouldNotInferId)?
                .to_string()
        } else {
            uri_str
        };
        Change::Add {
            id: Some(id_val),
            uri: Some(final_uri),
            flake: !no_flake,
        }
    } else if state.interactive {
        // Interactive mode
        let prefill_uri = id.as_deref();
        let tui_app = tui::App::add("Add", editor.text(), prefill_uri);
        let Some(tui::AppResult::Change(tui_change)) = tui::run(tui_app)? else {
            return Ok(()); // User cancelled
        };

        // Apply URI options if needed
        if ref_or_rev.is_some() || shallow {
            if let Change::Add {
                id,
                uri: Some(uri_str),
                flake,
            } = tui_change
            {
                let flake_ref: FlakeRef = uri_str
                    .parse()
                    .map_err(|e| CommandError::CouldNotInferId(format!("{}", e)))?;
                Change::Add {
                    id,
                    uri: Some(
                        apply_uri_options(flake_ref, ref_or_rev, shallow)
                            .map_err(CommandError::CouldNotInferId)?
                            .to_string(),
                    ),
                    flake: flake && !no_flake,
                }
            } else {
                tui_change
            }
        } else if no_flake {
            if let Change::Add { id, uri, .. } = tui_change {
                Change::Add {
                    id,
                    uri,
                    flake: false,
                }
            } else {
                tui_change
            }
        } else {
            tui_change
        }
    } else if let Some(uri) = id {
        // Non-interactive with URI provided positionally (id field is actually URI)
        let flake_ref: NixUriResult<FlakeRef> = UrlWrapper::convert_or_parse(&uri);

        let (inferred_id, final_uri) = if let Ok(flake_ref) = flake_ref {
            let flake_ref = apply_uri_options(flake_ref, ref_or_rev, shallow)
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

        Change::Add {
            id: Some(final_id),
            uri: Some(final_uri),
            flake: !no_flake,
        }
    } else {
        return Err(CommandError::NoUri);
    };

    apply_change(editor, flake_edit, state, change)
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
        let mut keys: Vec<_> = inputs.keys().collect();
        keys.sort();
        for input_id in keys {
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

    let change = if id.is_none() && uri.is_none() && state.interactive {
        // Full interactive: select input, then enter URI
        let mut keys: Vec<_> = inputs.keys().collect();
        keys.sort();
        let input_pairs: Vec<(String, String)> = keys
            .iter()
            .map(|id| {
                (
                    (*id).clone(),
                    inputs[*id].url().trim_matches('"').to_string(),
                )
            })
            .collect();
        if input_pairs.is_empty() {
            return Err(CommandError::NoInputs);
        }

        let tui_app = tui::App::change("Change", editor.text(), input_pairs);
        let Some(tui::AppResult::Change(tui_change)) = tui::run(tui_app)? else {
            return Ok(());
        };

        if ref_or_rev.is_some() || shallow {
            if let Change::Change {
                id,
                uri: Some(uri_str),
                ..
            } = tui_change
            {
                let flake_ref: FlakeRef = uri_str
                    .parse()
                    .map_err(|e| CommandError::CouldNotInferId(format!("{}", e)))?;
                Change::Change {
                    id,
                    uri: Some(
                        apply_uri_options(flake_ref, ref_or_rev, shallow)
                            .map_err(CommandError::CouldNotInferId)?
                            .to_string(),
                    ),
                    ref_or_rev: None,
                }
            } else {
                tui_change
            }
        } else {
            tui_change
        }
    } else if id.is_some() && uri.is_none() && state.interactive {
        // ID provided but no URI: show URI input
        let id_ref = id.as_ref().unwrap();
        let current_uri = inputs.get(id_ref).map(|i| i.url().trim_matches('"'));
        let tui_app =
            tui::App::change_uri("Change", editor.text(), id_ref, current_uri, state.diff);
        let Some(tui::AppResult::Change(tui_change)) = tui::run(tui_app)? else {
            return Ok(());
        };

        if let Change::Change {
            uri: Some(new_uri), ..
        } = tui_change
        {
            let final_uri = if ref_or_rev.is_some() || shallow {
                let flake_ref: FlakeRef = new_uri
                    .parse()
                    .map_err(|e| CommandError::CouldNotInferId(format!("{}", e)))?;
                apply_uri_options(flake_ref, ref_or_rev, shallow)
                    .map_err(CommandError::CouldNotInferId)?
                    .to_string()
            } else {
                new_uri
            };
            Change::Change {
                id,
                uri: Some(final_uri),
                ref_or_rev: None,
            }
        } else {
            return Err(CommandError::NoUri);
        }
    } else if let (Some(id_val), Some(uri_str)) = (id.clone(), uri) {
        // Both provided - non-interactive
        let final_uri = if ref_or_rev.is_some() || shallow {
            let flake_ref: FlakeRef = uri_str
                .parse()
                .map_err(|e| CommandError::CouldNotInferId(format!("{}: {}", uri_str, e)))?;
            apply_uri_options(flake_ref, ref_or_rev, shallow)
                .map_err(CommandError::CouldNotInferId)?
                .to_string()
        } else {
            uri_str
        };
        Change::Change {
            id: Some(id_val),
            uri: Some(final_uri),
            ref_or_rev: None,
        }
    } else if let Some(uri) = id {
        // Only positional arg provided, try to infer ID from URI
        let flake_ref: NixUriResult<FlakeRef> = UrlWrapper::convert_or_parse(&uri);
        if let Ok(flake_ref) = flake_ref {
            let flake_ref = apply_uri_options(flake_ref, ref_or_rev, shallow)
                .map_err(CommandError::CouldNotInferId)?;
            let final_uri = if flake_ref.to_string().is_empty() {
                uri.clone()
            } else {
                flake_ref.to_string()
            };
            if let Some(id) = flake_ref.id() {
                Change::Change {
                    id: Some(id),
                    uri: Some(final_uri),
                    ref_or_rev: None,
                }
            } else {
                return Err(CommandError::CouldNotInferId(uri));
            }
        } else {
            return Err(CommandError::CouldNotInferId(uri));
        }
    } else {
        return Err(CommandError::NoId);
    };

    apply_change(editor, flake_edit, state, change)
}

pub fn update(
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
    id: Option<String>,
    init: bool,
) -> Result<()> {
    let inputs = flake_edit.list().clone();
    let mut input_ids: Vec<String> = inputs.keys().cloned().collect();
    input_ids.sort();

    if let Some(id) = id {
        let mut updater = Updater::new(Rope::from_str(&editor.text()), inputs);
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

        loop {
            let select_app = tui::App::select_many(
                "Update",
                "Space select, U all, ^D diff",
                display_items.clone(),
                state.diff,
            );
            let Some(tui::AppResult::MultiSelect(result)) = tui::run(select_app)? else {
                return Ok(());
            };
            let tui::MultiSelectResultData {
                items: selected,
                show_diff,
            } = result;
            let ids: Vec<String> = selected
                .iter()
                .map(|s| s.split(" - ").next().unwrap_or(s).to_string())
                .collect();

            let mut updater = Updater::new(Rope::from_str(&editor.text()), inputs.clone());
            for id in &ids {
                updater.update_all_inputs_to_latest_semver(Some(id.clone()), init);
            }
            let change = updater.get_changes();

            match confirm_or_apply(editor, state, "Update", &change, show_diff)? {
                ConfirmResult::Applied => break,
                ConfirmResult::Back => continue,
                ConfirmResult::Cancelled => return Ok(()),
            }
        }
    } else {
        let mut updater = Updater::new(Rope::from_str(&editor.text()), inputs);
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
    let mut input_ids: Vec<String> = inputs.keys().cloned().collect();
    input_ids.sort();

    if let Some(id) = id {
        let lock = FlakeLock::from_default_path().map_err(|_| CommandError::NoLock)?;
        let target_rev = if let Some(rev) = rev {
            rev
        } else {
            lock.get_rev_by_id(&id)
                .map_err(|_| CommandError::InputNotFound(id.clone()))?
        };
        let mut updater = Updater::new(Rope::from_str(&editor.text()), inputs);
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

        loop {
            let select_app =
                tui::App::select_one("Pin", "Select input", input_ids.clone(), state.diff);
            let Some(tui::AppResult::SingleSelect(result)) = tui::run(select_app)? else {
                return Ok(());
            };
            let tui::SingleSelectResult {
                item: id,
                show_diff,
            } = result;
            let target_rev = lock
                .get_rev_by_id(&id)
                .map_err(|_| CommandError::InputNotFound(id.clone()))?;
            let mut updater = Updater::new(Rope::from_str(&editor.text()), inputs.clone());
            updater.pin_input_to_ref(&id, &target_rev);
            let change = updater.get_changes();

            match confirm_or_apply(editor, state, "Pin", &change, show_diff)? {
                ConfirmResult::Applied => {
                    println!("Pinned input: {} to {}", id, target_rev);
                    break;
                }
                ConfirmResult::Back => continue,
                ConfirmResult::Cancelled => return Ok(()),
            }
        }
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
    let mut input_ids: Vec<String> = inputs.keys().cloned().collect();
    input_ids.sort();

    if let Some(id) = id {
        let mut updater = Updater::new(Rope::from_str(&editor.text()), inputs);
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

        loop {
            let select_app =
                tui::App::select_one("Unpin", "Select input", input_ids.clone(), state.diff);
            let Some(tui::AppResult::SingleSelect(result)) = tui::run(select_app)? else {
                return Ok(());
            };
            let tui::SingleSelectResult {
                item: id,
                show_diff,
            } = result;
            let mut updater = Updater::new(Rope::from_str(&editor.text()), inputs.clone());
            updater.unpin_input(&id);
            let change = updater.get_changes();

            match confirm_or_apply(editor, state, "Unpin", &change, show_diff)? {
                ConfirmResult::Applied => {
                    println!("Unpinned input: {}", id);
                    break;
                }
                ConfirmResult::Back => continue,
                ConfirmResult::Cancelled => return Ok(()),
            }
        }
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

    let mut to_follow: Vec<(String, String)> = Vec::new();

    for nested in &ctx.nested_inputs {
        if nested.follows.is_some() {
            continue;
        }

        // e.g., "crane.nixpkgs" -> "nixpkgs"
        let nested_name = nested.path.split('.').next_back().unwrap_or(&nested.path);

        if ctx.top_level_inputs.contains(nested_name) {
            to_follow.push((nested.path.clone(), nested_name.to_string()));
        }
    }

    if to_follow.is_empty() {
        println!("No inputs to auto-follow.");
        println!(
            "All nested inputs either already follow something or have no matching top-level input."
        );
        return Ok(());
    }

    if state.diff {
        let mut current_text = editor.text();

        for (input_path, target) in &to_follow {
            let change = Change::Follows {
                input: input_path.clone().into(),
                target: target.clone(),
            };

            let mut temp_flake_edit =
                FlakeEdit::from_text(&current_text).map_err(CommandError::FlakeEdit)?;

            if let Ok(Some(resulting_change)) = temp_flake_edit.apply_change(change) {
                current_text = resulting_change;
            }
        }

        let original_text = editor.text();
        let diff = crate::diff::Diff::new(&original_text, &current_text);
        diff.compare();
    } else {
        let flake_path = editor.path().clone();
        let mut applied_count = 0;

        for (input_path, target) in &to_follow {
            let change = Change::Follows {
                input: input_path.clone().into(),
                target: target.clone(),
            };

            // Re-read from disk to get fresh state after previous writes
            let fresh_editor = Editor::from_path(flake_path.clone())?;
            let mut fresh_flake_edit = fresh_editor
                .create_flake_edit()
                .map_err(CommandError::FlakeEdit)?;

            match fresh_flake_edit.apply_change(change) {
                Ok(Some(resulting_change)) => {
                    let root = rnix::Root::parse(&resulting_change);
                    let errors = root.errors();
                    if !errors.is_empty() {
                        eprintln!("Error applying follows for {}: parse errors", input_path);
                        for e in errors {
                            tracing::error!("Error: {e}");
                        }
                        continue;
                    }

                    fresh_editor.apply_or_diff(&resulting_change, state)?;
                    applied_count += 1;

                    let nested_name = input_path.split('.').next_back().unwrap_or(input_path);
                    let parent = input_path.split('.').next().unwrap_or(input_path);
                    println!(
                        "Added follows: {}.inputs.{}.follows = \"{}\"",
                        parent, nested_name, target
                    );
                }
                Ok(None) => {
                    eprintln!("Could not create follows for {}", input_path);
                }
                Err(e) => {
                    eprintln!("Error applying follows for {}: {}", input_path, e);
                }
            }
        }

        if applied_count > 0 {
            println!("\nAuto-followed {} input(s).", applied_count);
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
                match &change {
                    Change::Add { id, uri, .. } => {
                        // Cache the entry for future completions
                        if let (Some(id), Some(uri)) = (id, uri) {
                            let mut cache = crate::cache::Cache::load();
                            cache.add_entry(id.clone(), uri.clone());
                            if let Err(e) = cache.commit() {
                                tracing::debug!("Could not write to cache: {}", e);
                            }
                        }
                        println!(
                            "Added input: {} = {}",
                            id.as_deref().unwrap_or("?"),
                            uri.as_deref().unwrap_or("?")
                        );
                    }
                    Change::Remove { ids } => {
                        for id in ids {
                            println!("Removed input: {}", id);
                        }
                    }
                    Change::Change { id, uri, .. } => {
                        println!(
                            "Changed input: {} -> {}",
                            id.as_deref().unwrap_or("?"),
                            uri.as_deref().unwrap_or("?")
                        );
                    }
                    Change::Follows { input, target } => {
                        println!(
                            "Added follows: {}.inputs.{}.follows = \"{}\"",
                            input.input(),
                            input.follows().unwrap_or("?"),
                            target
                        );
                    }
                    _ => {}
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
