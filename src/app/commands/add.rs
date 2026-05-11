//! `flake-edit add`: append a new input to the flake.
//!
//! Three branches: scripted (id + uri), interactive TUI (with
//! optional prefill), and infer-id (uri only, ID derived from the
//! parsed [`FlakeRef`]).

use nix_uri::FlakeRef;

use crate::change::Change;
use crate::edit::FlakeEdit;
use crate::tui;

use super::super::editor::Editor;
use super::super::state::AppState;
use super::uri::{BuildKind, UriOptions, apply_uri_options, build_uri_change, transform_uri};
use super::{Error, Result, apply_change};

pub fn add(
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
    id: Option<String>,
    uri: Option<String>,
    no_flake: bool,
    opts: UriOptions<'_>,
) -> Result<()> {
    let change = match (id, uri, state.interactive) {
        // Both ID and URI provided: non-interactive add.
        (Some(id_val), Some(uri_str), _) => {
            build_uri_change(BuildKind::Add { no_flake }, id_val, uri_str, &opts)?
        }
        // Interactive: show TUI (with or without prefill).
        (id, None, true) | (None, id, true) => {
            add_interactive(editor, state, id.as_deref(), no_flake, &opts)?
        }
        // Non-interactive with only one positional arg: infer ID from URI.
        (Some(uri), None, false) | (None, Some(uri), false) => add_infer_id(uri, no_flake, &opts)?,
        (None, None, false) => {
            return Err(Error::NoUri);
        }
    };

    apply_change(editor, flake_edit, state, change)
}

fn add_interactive(
    editor: &Editor,
    state: &AppState,
    prefill_uri: Option<&str>,
    no_flake: bool,
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
            flake: flake && !no_flake,
        })
    } else {
        Ok(tui_change)
    }
}

/// Builds a `Change::Add` when only the URI is supplied, inferring
/// the ID from the parsed flake reference.
fn add_infer_id(uri: String, no_flake: bool, opts: &UriOptions<'_>) -> Result<Change> {
    let (inferred_id, final_uri) = match uri.parse::<FlakeRef>() {
        Ok(flake_ref) => {
            let flake_ref = apply_uri_options(flake_ref, opts.ref_or_rev, opts.shallow);
            let id = flake_ref.id().map(str::to_owned);
            (id, flake_ref.into_uri())
        }
        Err(_) => (None, uri.clone()),
    };

    let final_id = inferred_id.ok_or_else(|| Error::CouldNotInferId { uri: uri.clone() })?;

    Ok(Change::Add {
        id: Some(final_id),
        uri: Some(final_uri),
        flake: !no_flake,
    })
}
