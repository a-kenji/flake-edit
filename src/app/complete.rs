//! Dynamic shell completions.

use clap::CommandFactory;
use clap_complete::CompleteEnv;
use clap_complete::engine::{ArgValueCandidates, CompletionCandidate};

use crate::cache::{Cache, DEFAULT_URI_TYPES};
use crate::cli::CliArgs;
use crate::edit::{FlakeEdit, sorted_input_ids};
use crate::input::Follows;

use super::editor::Editor;
use super::handler::root::Root;

/// Handle a completion request and exit, or return for a normal run.
///
/// Must be called before any other output.
pub fn complete() {
    CompleteEnv::with_factory(command).complete();
}

fn command() -> clap::Command {
    let mut cmd = CliArgs::command();
    for &(sub, arg, candidates) in COMPLETERS {
        cmd = attach(cmd, sub, arg, candidates);
    }
    cmd
}

type Completer = fn() -> Vec<CompletionCandidate>;

/// Mapping of dynamic completions to their trigger.
const COMPLETERS: &[(&str, &str, Completer)] = &[
    ("remove", "id", input_candidates),
    ("change", "id", input_candidates),
    ("pin", "id", toplevel_candidates),
    ("unpin", "id", toplevel_candidates),
    ("update", "id", toplevel_candidates),
    ("toggle", "input", toggle_candidates),
    ("add-follow", "input", follow_candidates),
    ("add-follow", "target", toplevel_candidates),
    ("add", "id", uri_candidates),
    ("add", "uri", uri_candidates),
];

/// Attach `candidates` to `arg` of `sub` without reordering its arguments.
fn attach(
    cmd: clap::Command,
    sub: &str,
    arg: &'static str,
    candidates: Completer,
) -> clap::Command {
    cmd.mut_subcommand(sub, move |c| {
        c.mut_args(move |a| {
            if a.get_id().as_str() == arg {
                a.add(ArgValueCandidates::new(candidates))
            } else {
                a
            }
        })
    })
}

/// Load the current directory's flake.{nix,lock}.
///
/// This is the source of many dynamic completions.
fn load() -> Option<FlakeEdit> {
    let root = Root::from_path("flake.nix").ok()?;
    let editor = Editor::from_path(root.path().to_path_buf()).ok()?;
    editor.create_flake_edit().ok()
}

fn input_candidates() -> Vec<CompletionCandidate> {
    let Some(mut flake_edit) = load() else {
        return Vec::new();
    };
    let inputs = flake_edit.list();
    let mut out = Vec::new();
    for key in sorted_input_ids(inputs) {
        let input = &inputs[key];
        out.push(CompletionCandidate::new(input.id().as_str()));
        for follows in input.follows() {
            if let Follows::Indirect { path, .. } = follows {
                out.push(CompletionCandidate::new(format!(
                    "{}.{}",
                    input.id().as_str(),
                    path
                )));
            }
        }
    }
    out
}

fn toplevel_candidates() -> Vec<CompletionCandidate> {
    let Some(mut flake_edit) = load() else {
        return Vec::new();
    };
    let inputs = flake_edit.list();
    sorted_input_ids(inputs)
        .into_iter()
        .map(|id| CompletionCandidate::new(id.as_str()))
        .collect()
}

/// Known URI scheme prefixes plus previously seen URIs from the cache.
fn uri_candidates() -> Vec<CompletionCandidate> {
    let mut out: Vec<CompletionCandidate> = DEFAULT_URI_TYPES
        .iter()
        .map(|uri_type| CompletionCandidate::new(*uri_type))
        .collect();
    for uri in Cache::load().list_uris() {
        out.push(CompletionCandidate::new(uri));
    }
    out
}

/// Nested input paths discovered in `flake.lock`, for manual follows targets.
fn follow_candidates() -> Vec<CompletionCandidate> {
    let Ok(lock) = crate::lock::FlakeLock::from_default_path() else {
        return Vec::new();
    };
    lock.nested_inputs()
        .into_iter()
        .map(|nested| CompletionCandidate::new(nested.path.to_string()))
        .collect()
}

fn toggle_candidates() -> Vec<CompletionCandidate> {
    let Some(mut flake_edit) = load() else {
        return Vec::new();
    };
    let Ok(states) = flake_edit.toggle_states() else {
        return Vec::new();
    };
    states
        .into_iter()
        .filter(|(_, state)| !state.alternates.is_empty())
        .map(|(id, _)| CompletionCandidate::new(id))
        .collect()
}
