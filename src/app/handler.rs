use std::path::PathBuf;

use crate::cli::{CliArgs, Command};
use crate::edit::FlakeEdit;
use crate::tui;

use super::commands::follow;
use super::commands::{self};
use super::editor::Editor;
use super::error::{Error, Result};
use super::state::AppState;

mod root;

/// Application entry point.
///
/// Parses CLI arguments, initializes state, and dispatches to command handlers.
pub fn run(args: CliArgs) -> Result<()> {
    if let Command::Follow {
        paths,
        transitive,
        depth,
    } = args.subcommand()
        && !paths.is_empty()
    {
        if args.flake().is_some() || args.lock_file().is_some() {
            return Err(Error::IncompatibleFollowOptions);
        }
        return follow::auto::run_batch(paths, *transitive, *depth, &args);
    }

    let (editor, mut flake_edit, mut state) = setup(&args)?;
    let no_cache = args.no_cache();

    match args.subcommand() {
        Command::Add { .. } => dispatch_add(&args, &editor, &mut flake_edit, &state)?,
        Command::Remove { .. } => dispatch_remove(&args, &editor, &mut flake_edit, &state)?,
        Command::Change { .. } => dispatch_change(&args, &editor, &mut flake_edit, &state)?,
        Command::List { .. } => dispatch_list(&args, &mut flake_edit)?,
        Command::Update { .. } => dispatch_update(&args, &editor, &mut flake_edit, &state)?,
        Command::Pin { .. } => dispatch_pin(&args, &editor, &mut flake_edit, &state)?,
        Command::Unpin { .. } => dispatch_unpin(&args, &editor, &mut flake_edit, &state)?,
        Command::Toggle { .. } => dispatch_toggle(&args, &editor, &mut flake_edit, &state)?,
        Command::Follow { .. } => dispatch_follow(&args, &editor, &mut flake_edit, &mut state)?,
        Command::AddFollow { .. } => {
            dispatch_add_follow(&args, &editor, &mut flake_edit, &mut state)?
        }
        Command::Completion { .. } => {
            return dispatch_completion(&args, &mut flake_edit, no_cache);
        }
        Command::Config { .. } => return dispatch_config(&args),
    }

    crate::cache::populate_cache_from_input_map(flake_edit.curr_list(), no_cache);

    Ok(())
}

fn setup(args: &CliArgs) -> Result<(Editor, FlakeEdit, AppState)> {
    let flake_path = if let Some(flake) = args.flake() {
        let path = PathBuf::from(flake);
        if path.is_dir() {
            let flake_nix = path.join("flake.nix");
            if !flake_nix.exists() {
                return Err(Error::FlakeDirEmpty { path });
            }
            flake_nix
        } else {
            path
        }
    } else {
        let path = PathBuf::from("flake.nix");
        let binding = root::Root::from_path(&path).map_err(|source| Error::FlakeNotFound {
            path: path.clone(),
            source,
        })?;
        binding.path().to_path_buf()
    };

    let editor = Editor::from_path(flake_path.clone()).map_err(|source| Error::FlakeNotFound {
        path: flake_path.clone(),
        source,
    })?;
    let flake_edit = editor.create_flake_edit()?;
    let interactive = tui::is_interactive(args.non_interactive());

    let state = AppState::new(flake_path, args.config().map(PathBuf::from))?
        .with_diff(args.diff())
        .with_no_lock(args.no_lock())
        .with_interactive(interactive)
        .with_lock_file(args.lock_file().map(PathBuf::from))
        .with_no_cache(args.no_cache())
        .with_cache_path(args.cache().map(PathBuf::from));

    Ok((editor, flake_edit, state))
}

fn dispatch_add(
    args: &CliArgs,
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
) -> Result<()> {
    let Command::Add {
        uri,
        ref_or_rev,
        id,
        no_flake,
        shallow,
    } = args.subcommand()
    else {
        unreachable!("wrong Command variant");
    };
    commands::add(
        editor,
        flake_edit,
        state,
        id.clone(),
        uri.clone(),
        *no_flake,
        commands::UriOptions {
            ref_or_rev: ref_or_rev.as_deref(),
            shallow: *shallow,
        },
    )
}

fn dispatch_remove(
    args: &CliArgs,
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
) -> Result<()> {
    let Command::Remove { id } = args.subcommand() else {
        unreachable!("wrong Command variant");
    };
    commands::remove(editor, flake_edit, state, id.clone())
}

fn dispatch_change(
    args: &CliArgs,
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
) -> Result<()> {
    let Command::Change {
        uri,
        ref_or_rev,
        id,
        shallow,
    } = args.subcommand()
    else {
        unreachable!("wrong Command variant");
    };
    commands::change(
        editor,
        flake_edit,
        state,
        id.clone(),
        uri.clone(),
        commands::UriOptions {
            ref_or_rev: ref_or_rev.as_deref(),
            shallow: *shallow,
        },
    )
}

fn dispatch_list(args: &CliArgs, flake_edit: &mut FlakeEdit) -> Result<()> {
    let Command::List { format } = args.subcommand() else {
        unreachable!("wrong Command variant");
    };
    commands::list(flake_edit, format)
}

fn dispatch_update(
    args: &CliArgs,
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
) -> Result<()> {
    let Command::Update { id, init } = args.subcommand() else {
        unreachable!("wrong Command variant");
    };
    commands::update(editor, flake_edit, state, id.clone(), *init)
}

fn dispatch_pin(
    args: &CliArgs,
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
) -> Result<()> {
    let Command::Pin { id, rev } = args.subcommand() else {
        unreachable!("wrong Command variant");
    };
    commands::pin(editor, flake_edit, state, id.clone(), rev.clone())
}

fn dispatch_unpin(
    args: &CliArgs,
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
) -> Result<()> {
    let Command::Unpin { id } = args.subcommand() else {
        unreachable!("wrong Command variant");
    };
    commands::unpin(editor, flake_edit, state, id.clone())
}

fn dispatch_toggle(
    args: &CliArgs,
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
) -> Result<()> {
    let Command::Toggle {
        input,
        reference,
        remove,
    } = args.subcommand()
    else {
        unreachable!("wrong Command variant");
    };
    commands::toggle(
        editor,
        flake_edit,
        state,
        input.clone(),
        reference.clone(),
        *remove,
    )
}

fn dispatch_follow(
    args: &CliArgs,
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &mut AppState,
) -> Result<()> {
    let Command::Follow {
        paths: _,
        transitive,
        depth,
    } = args.subcommand()
    else {
        unreachable!("wrong Command variant");
    };
    if let Some(min) = transitive {
        state.config.follow.transitive_min = *min;
    }
    if let Some(max) = depth {
        state.config.follow.max_depth = Some(*max);
    }
    state.lock_offline = true;
    follow::auto::run(editor, flake_edit, state)
}

fn dispatch_add_follow(
    args: &CliArgs,
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &mut AppState,
) -> Result<()> {
    let Command::AddFollow { input, target } = args.subcommand() else {
        unreachable!("wrong Command variant");
    };
    state.lock_offline = true;
    follow::add_follow(editor, flake_edit, state, input.clone(), target.clone())
}

fn dispatch_completion(args: &CliArgs, flake_edit: &mut FlakeEdit, no_cache: bool) -> Result<()> {
    use crate::cache::{Cache, DEFAULT_URI_TYPES};
    use crate::cli::CompletionMode;

    let Command::Completion { inputs: _, mode } = args.subcommand() else {
        unreachable!("wrong Command variant");
    };
    match mode {
        CompletionMode::Add => {
            for uri_type in DEFAULT_URI_TYPES {
                println!("{}", uri_type);
            }
            let cache = Cache::load();
            for uri in cache.list_uris() {
                println!("{}", uri);
            }
        }
        CompletionMode::Change => {
            let inputs = flake_edit.list();
            crate::cache::populate_cache_from_input_map(inputs, no_cache);
            for id in inputs.keys() {
                println!("{}", id);
            }
        }
        CompletionMode::Follow => {
            if let Ok(lock) = crate::lock::FlakeLock::from_default_path() {
                for nested in lock.nested_inputs() {
                    println!("{}", nested.path);
                }
            }
        }
        CompletionMode::Toggle => {
            let states = flake_edit.toggle_states()?;
            for (id, state) in states {
                if !state.alternates.is_empty() {
                    println!("{}", id);
                }
            }
        }
    }
    Ok(())
}

fn dispatch_config(args: &CliArgs) -> Result<()> {
    let Command::Config {
        print_default,
        path,
    } = args.subcommand()
    else {
        unreachable!("wrong Command variant");
    };
    commands::config(*print_default, *path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    const MINIMAL_FLAKE: &str = "{\n  inputs = {};\n  outputs = { self }: { };\n}\n";

    fn write_minimal_flake(dir: &std::path::Path) -> std::path::PathBuf {
        let path = dir.join("flake.nix");
        std::fs::write(&path, MINIMAL_FLAKE).expect("write flake.nix");
        path
    }

    fn parse(args: &[&str]) -> CliArgs {
        CliArgs::try_parse_from(args).expect("parse CLI args")
    }

    #[test]
    fn batch_follow_with_flake_flag_is_rejected() {
        let args = parse(&[
            "flake-edit",
            "--flake",
            "/does/not/exist/flake.nix",
            "follow",
            "/some/path/flake.nix",
        ]);
        let err = run(args).expect_err("batch follow + --flake must be rejected");
        assert!(matches!(err, Error::IncompatibleFollowOptions));
    }

    #[test]
    fn batch_follow_with_lock_file_flag_is_rejected() {
        let args = parse(&[
            "flake-edit",
            "--lock-file",
            "/does/not/exist/flake.lock",
            "follow",
            "/some/path/flake.nix",
        ]);
        let err = run(args).expect_err("batch follow + --lock-file must be rejected");
        assert!(matches!(err, Error::IncompatibleFollowOptions));
    }

    #[test]
    fn flake_dir_without_flake_nix_returns_flake_dir_empty() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let args = parse(&[
            "flake-edit",
            "--flake",
            tmp.path().to_str().unwrap(),
            "list",
        ]);
        let err = run(args).expect_err("empty dir must yield FlakeDirEmpty");
        assert!(matches!(err, Error::FlakeDirEmpty { .. }));
    }

    #[test]
    fn missing_flake_file_returns_flake_not_found() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let missing = tmp.path().join("missing.nix");
        let args = parse(&["flake-edit", "--flake", missing.to_str().unwrap(), "list"]);
        let err = run(args).expect_err("missing file must yield FlakeNotFound");
        assert!(matches!(err, Error::FlakeNotFound { .. }));
    }

    #[test]
    fn config_print_default_does_not_touch_flake_nix() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let flake = write_minimal_flake(tmp.path());
        let args = parse(&[
            "flake-edit",
            "--flake",
            tmp.path().to_str().unwrap(),
            "--non-interactive",
            "--no-cache",
            "config",
            "--print-default",
        ]);
        run(args).expect("config --print-default must succeed");
        assert_eq!(
            std::fs::read_to_string(&flake).expect("read flake.nix"),
            MINIMAL_FLAKE,
            "config --print-default must not rewrite flake.nix",
        );
    }

    #[test]
    fn completion_change_does_not_touch_flake_nix() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let flake = write_minimal_flake(tmp.path());
        let args = parse(&[
            "flake-edit",
            "--flake",
            tmp.path().to_str().unwrap(),
            "--non-interactive",
            "--no-cache",
            "completion",
            "change",
        ]);
        run(args).expect("completion change must succeed");
        assert_eq!(
            std::fs::read_to_string(&flake).expect("read flake.nix"),
            MINIMAL_FLAKE,
            "completion change must not rewrite flake.nix",
        );
    }
}
