use std::path::PathBuf;

use crate::cli::{CliArgs, Command};
use crate::config::ConfigError;
use crate::tui;

use super::commands::follow;
use super::commands::{self, CommandError};
use super::editor::Editor;
use super::state::AppState;

mod root;

pub type Result<T> = std::result::Result<T, HandlerError>;

#[derive(Debug, thiserror::Error)]
pub enum HandlerError {
    #[error(transparent)]
    Command(#[from] CommandError),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    FlakeEdit(#[from] crate::error::FlakeEditError),

    #[error(transparent)]
    Config(#[from] ConfigError),

    #[error("Flake not found")]
    FlakeNotFound,

    #[error("--flake and --lock cannot be used with 'follow [PATHS]'")]
    IncompatibleFollowOptions,
}

/// Application entry point.
///
/// Parses CLI arguments, initializes state, and dispatches to command handlers.
pub fn run(args: CliArgs) -> Result<()> {
    // Batch `follow [PATHS...]` runs before creating Editor/AppState because
    // it owns its own per-file Editor/AppState pairs.
    if let Command::Follow {
        paths,
        transitive,
        depth,
    } = args.subcommand()
        && !paths.is_empty()
    {
        if args.flake().is_some() || args.lock_file().is_some() {
            return Err(HandlerError::IncompatibleFollowOptions);
        }
        return follow::auto::run_batch(paths, *transitive, *depth, &args)
            .map_err(HandlerError::Command);
    }

    let flake_path = if let Some(flake) = args.flake() {
        let path = PathBuf::from(flake);
        if path.is_dir() {
            let flake_nix = path.join("flake.nix");
            if !flake_nix.exists() {
                return Err(HandlerError::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("No `flake.nix` found in directory: {}", path.display()),
                )));
            }
            flake_nix
        } else {
            path
        }
    } else {
        let path = PathBuf::from("flake.nix");
        let binding = root::Root::from_path(path).map_err(|_| HandlerError::FlakeNotFound)?;
        binding.path().to_path_buf()
    };

    let editor = Editor::from_path(flake_path.clone())?;
    let mut flake_edit = editor.create_flake_edit()?;
    let interactive = tui::is_interactive(args.non_interactive());

    let no_cache = args.no_cache();
    let mut state = AppState::new(editor.text(), flake_path, args.config().map(PathBuf::from))?
        .with_diff(args.diff())
        .with_no_lock(args.no_lock())
        .with_interactive(interactive)
        .with_lock_file(args.lock_file().map(PathBuf::from))
        .with_no_cache(no_cache)
        .with_cache_path(args.cache().map(PathBuf::from));

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
                *no_flake,
                commands::UriOptions {
                    ref_or_rev: ref_or_rev.as_deref(),
                    shallow: *shallow,
                },
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
                commands::UriOptions {
                    ref_or_rev: ref_or_rev.as_deref(),
                    shallow: *shallow,
                },
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

        Command::Follow {
            paths: _,
            transitive,
            depth,
        } => {
            // The batch path is handled above. This branch runs on the current flake.
            if let Some(min) = transitive {
                state.config.follow.transitive_min = *min;
            }
            if let Some(max) = depth {
                state.config.follow.max_depth = *max;
            }
            state.lock_offline = true;
            follow::auto::run(&editor, &mut flake_edit, &state)?;
        }

        Command::AddFollow { input, target } => {
            state.lock_offline = true;
            follow::add_follow(
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
                    crate::cache::populate_cache_from_input_map(inputs, no_cache);
                    for id in inputs.keys() {
                        println!("{}", id);
                    }
                    std::process::exit(0);
                }
                CompletionMode::Follow => {
                    if let Ok(lock) = crate::lock::FlakeLock::from_default_path() {
                        for path in lock.nested_input_paths() {
                            println!("{}", path);
                        }
                    }
                    std::process::exit(0);
                }
                CompletionMode::None => {}
            }
        }

        Command::Config {
            print_default,
            path,
        } => {
            commands::config(*print_default, *path)?;
            return Ok(());
        }
    }

    // Build up the completion cache as users interact with different flakes,
    // not only when they add inputs explicitly.
    crate::cache::populate_cache_from_input_map(flake_edit.curr_list(), no_cache);

    Ok(())
}
