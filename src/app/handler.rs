use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::cli::{CliArgs, Command, ListFormat};
use crate::edit::InputMap;
use crate::input::Follows;
use crate::tui;

use super::commands::{self, CommandError};
use super::editor::Editor;
use super::state::AppState;

mod root;

pub type Result<T> = std::result::Result<T, HandlerError>;

#[derive(Debug, thiserror::Error)]
pub enum HandlerError {
    #[error("{0}")]
    Command(#[from] CommandError),

    #[error("{0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    FlakeEdit(#[from] crate::error::FlakeEditError),

    #[error("Flake not found")]
    FlakeNotFound,
}

/// Main entry point for the application.
///
/// Parses CLI arguments, initializes state, and dispatches to command handlers.
pub fn run(args: CliArgs) -> Result<()> {
    // Find flake.nix path
    let flake_path = if let Some(flake) = args.flake() {
        PathBuf::from(flake)
    } else {
        let path = PathBuf::from("flake.nix");
        let binding = root::Root::from_path(path).map_err(|_| HandlerError::FlakeNotFound)?;
        binding.path().to_path_buf()
    };

    // Create editor and state
    let editor = Editor::from_path(flake_path.clone())?;
    let mut flake_edit = editor.create_flake_edit()?;
    let interactive = tui::is_interactive(args.non_interactive());

    let state = AppState::new(editor.text(), flake_path)
        .with_diff(args.diff())
        .with_no_lock(args.no_lock())
        .with_interactive(interactive);

    // Dispatch to command
    match args.subcommand() {
        Command::Add {
            uri,
            ref_or_rev,
            id,
            no_flake,
            shallow,
        } => {
            commands::add(
                &editor,
                &mut flake_edit,
                &state,
                id.clone(),
                uri.clone(),
                ref_or_rev.as_deref(),
                *no_flake,
                *shallow,
            )?;
        }

        Command::Remove { id } => {
            commands::remove(&editor, &mut flake_edit, &state, id.clone())?;
        }

        Command::Change {
            uri,
            ref_or_rev,
            id,
            shallow,
        } => {
            commands::change(
                &editor,
                &mut flake_edit,
                &state,
                id.clone(),
                uri.clone(),
                ref_or_rev.as_deref(),
                *shallow,
            )?;
        }

        Command::List { format } => {
            commands::list(&mut flake_edit, format)?;
        }

        Command::Update { id, init } => {
            commands::update(&editor, &mut flake_edit, &state, id.clone(), *init)?;
        }

        Command::Pin { id, rev } => {
            commands::pin(&editor, &mut flake_edit, &state, id.clone(), rev.clone())?;
        }

        Command::Unpin { id } => {
            commands::unpin(&editor, &mut flake_edit, &state, id.clone())?;
        }

        Command::Follow { input, target } => {
            commands::follow(
                &editor,
                &mut flake_edit,
                &state,
                input.clone(),
                target.clone(),
            )?;
        }

        Command::Completion { inputs: _, mode } => {
            use crate::cache::{Cache, DEFAULT_URI_TYPES};
            use crate::cli::CompletionMode;
            match mode {
                CompletionMode::Add => {
                    for uri_type in DEFAULT_URI_TYPES {
                        println!("{}", uri_type);
                    }
                    let cache = Cache::load();
                    for uri in cache.list_uris() {
                        println!("{}", uri);
                    }
                    std::process::exit(0);
                }
                CompletionMode::Change => {
                    let inputs = flake_edit.list();
                    for id in inputs.keys() {
                        println!("{}", id);
                    }
                    std::process::exit(0);
                }
                CompletionMode::Follow => {
                    // Get nested input paths from lockfile for follow completions
                    if let Ok(lock) = crate::lock::FlakeLock::from_default_path() {
                        for path in lock.get_nested_input_paths() {
                            println!("{}", path);
                        }
                    }
                    std::process::exit(0);
                }
                CompletionMode::None => {}
            }
        }
    }

    Ok(())
}

/// List inputs in the specified format.
pub fn list_inputs(inputs: &InputMap, format: &ListFormat) {
    match format {
        ListFormat::Simple => list_simple(inputs),
        ListFormat::Json => list_json(inputs),
        ListFormat::Detailed => list_detailed(inputs),
        ListFormat::Raw => list_raw(inputs),
        ListFormat::Toplevel => list_toplevel(inputs),
        ListFormat::None => unreachable!("Should not be possible"),
    }
}

fn list_simple(inputs: &InputMap) {
    let mut buf = String::new();
    let mut keys: Vec<_> = inputs.keys().collect();
    keys.sort();
    for key in keys {
        let input = &inputs[key];
        if !buf.is_empty() {
            buf.push('\n');
        }
        buf.push_str(input.id());
        for follows in input.follows() {
            if let Follows::Indirect(id, _) = follows {
                let id = format!("{}.{}", input.id(), id);
                if !buf.is_empty() {
                    buf.push('\n');
                }
                buf.push_str(&id);
            }
        }
    }
    println!("{buf}");
}

fn list_json(inputs: &InputMap) {
    let sorted: BTreeMap<_, _> = inputs.iter().collect();
    let json = serde_json::to_string(&sorted).unwrap();
    println!("{json}");
}

fn list_toplevel(inputs: &InputMap) {
    let mut buf = String::new();
    let mut keys: Vec<_> = inputs.keys().collect();
    keys.sort();
    for key in keys {
        if !buf.is_empty() {
            buf.push('\n');
        }
        buf.push_str(&key.to_string());
    }
    println!("{buf}");
}

fn list_raw(inputs: &InputMap) {
    let sorted: BTreeMap<_, _> = inputs.iter().collect();
    println!("{:#?}", sorted);
}

fn list_detailed(inputs: &InputMap) {
    let mut buf = String::new();
    let mut keys: Vec<_> = inputs.keys().collect();
    keys.sort();
    for key in keys {
        let input = &inputs[key];
        if !buf.is_empty() {
            buf.push('\n');
        }
        let id = format!("· {} - {}", input.id(), input.url());
        buf.push_str(&id);
        for follows in input.follows() {
            if let Follows::Indirect(id, follow_id) = follows {
                let id = format!("{}{} => {}", " ".repeat(5), id, follow_id);
                if !buf.is_empty() {
                    buf.push('\n');
                }
                buf.push_str(&id);
            }
        }
    }
    println!("{buf}");
}
