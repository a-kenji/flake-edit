//! `flake-edit change`: replace an input's URI in place.
//!
//! Four branches: full interactive (pick + URI), URI-only
//! interactive (with the ID known), scripted (id + uri), and
//! infer-id (uri only). All route the resulting URI through
//! [`super::uri::transform_uri`].

use nix_uri::FlakeRef;

use crate::change::{Change, ChangeId};
use crate::edit::{FlakeEdit, InputMap, sorted_input_ids};
use crate::tui;

use super::super::editor::Editor;
use super::super::state::AppState;
use super::uri::{BuildKind, UriOptions, apply_uri_options, build_uri_change, transform_uri};
use super::{Error, Result, apply_change};

pub fn change(
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
    id: Option<String>,
    uri: Option<String>,
    opts: UriOptions<'_>,
) -> Result<()> {
    let inputs = flake_edit.list();

    let change = match (id, uri, state.interactive) {
        // Full interactive: select input, then enter URI. Also covers the
        // case where only URI was provided interactively (need to select input).
        (None, None, true) | (None, Some(_), true) => {
            change_full_interactive(editor, state, inputs, &opts)?
        }
        // ID provided, no URI, interactive: show URI input for that ID.
        (Some(id), None, true) => change_uri_interactive(editor, state, inputs, &id, &opts)?,
        // Both ID and URI provided: non-interactive.
        (Some(id_val), Some(uri_str), _) => {
            build_uri_change(BuildKind::Change, id_val, uri_str, &opts)?
        }
        // Only one positional arg: infer ID from URI.
        (Some(uri), None, false) | (None, Some(uri), false) => change_infer_id(uri, &opts)?,
        (None, None, false) => {
            return Err(Error::NoId);
        }
    };

    apply_change(editor, flake_edit, state, change)
}

/// Runs the full interactive flow: pick an input from the list, then
/// enter the new URI.
fn change_full_interactive(
    editor: &Editor,
    state: &AppState,
    inputs: &InputMap,
    opts: &UriOptions<'_>,
) -> Result<Change> {
    let input_pairs: Vec<(String, String)> = sorted_input_ids(inputs)
        .into_iter()
        .map(|id| (id.clone(), inputs[id].url().to_string()))
        .collect();

    if input_pairs.is_empty() {
        return Err(Error::NoInputs);
    }

    let tui_app = tui::App::change("Change", editor.text(), input_pairs, state.cache_config());
    let Some(tui::AppResult::Change(tui_change)) = tui::run(tui_app)? else {
        return Ok(Change::None);
    };

    // CLI options override the TUI result.
    if let Change::Change { id, uri, .. } = tui_change {
        let final_uri = uri
            .map(|u| transform_uri(u, opts.ref_or_rev, opts.shallow))
            .transpose()?;
        Ok(Change::Change { id, uri: final_uri })
    } else {
        Ok(tui_change)
    }
}

/// Runs the interactive flow with the ID already known, showing only
/// the URI input widget.
fn change_uri_interactive(
    editor: &Editor,
    state: &AppState,
    inputs: &InputMap,
    id: &str,
    opts: &UriOptions<'_>,
) -> Result<Change> {
    let current_uri = inputs.get(id).map(|i| i.url());
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
        let final_uri = transform_uri(new_uri, opts.ref_or_rev, opts.shallow)?;
        let id = ChangeId::parse(id).map_err(|source| Error::InvalidInputId {
            id: id.to_string(),
            source,
        })?;
        Ok(Change::Change {
            id: Some(id),
            uri: Some(final_uri),
        })
    } else {
        Err(Error::NoUri)
    }
}

/// Builds a `Change::Change` when only the URI is supplied, inferring
/// the ID from the parsed flake reference.
fn change_infer_id(uri: String, opts: &UriOptions<'_>) -> Result<Change> {
    let flake_ref: FlakeRef = uri.parse().map_err(|source| Error::InvalidUri {
        uri: uri.clone(),
        source,
    })?;
    let flake_ref = apply_uri_options(flake_ref, opts.ref_or_rev, opts.shallow);

    let id = flake_ref
        .id()
        .map(str::to_owned)
        .ok_or_else(|| Error::CouldNotInferId { uri: uri.clone() })?;
    let id = ChangeId::parse(&id).map_err(|source| Error::InvalidInputId { id, source })?;
    let final_uri = flake_ref.into_uri();

    Ok(Change::Change {
        id: Some(id),
        uri: Some(final_uri),
    })
}
