//! Will look in the following manner for a `flake.nix` file:
//!     - In the cwd
//!     - In the directory upwards to `git_root`
//!
use crate::app::FlakeEdit;
use crate::cli::CliArgs;
use crate::cli::Command;
use clap::Parser;
use color_eyre::Section;
use color_eyre::eyre;
use flake_edit::change::Change;
use flake_edit::edit;
use flake_edit::lock::FlakeLock;
use flake_edit::update::Updater;
use list::list_inputs;
use nix_uri::urls::UrlWrapper;
use nix_uri::{FlakeRef, NixUriResult};

mod app;
mod cache;
mod cli;
mod error;
mod list;
mod log;
mod root;

fn main() -> eyre::Result<()> {
    let args = CliArgs::parse();
    color_eyre::install()?;
    log::init().ok();
    tracing::debug!("Cli args: {args:?}");

    let app = FlakeEdit::init(&args)?;
    let mut editor = app.create_editor()?;
    let mut change = Change::None;

    match args.subcommand() {
        cli::Command::Add {
            uri,
            ref_or_rev: _,
            id,
            no_flake,
        } => {
            if id.is_some() && uri.is_some() {
                change = Change::Add {
                    id: id.clone(),
                    uri: uri.clone(),
                    flake: !no_flake,
                };
            } else if let Some(uri) = id {
                tracing::debug!("No [ID] provided trying to parse [uri] to infer [ID].");
                let flake_ref: NixUriResult<FlakeRef> = UrlWrapper::convert_or_parse(uri);
                tracing::debug!("The parsed flake reference is: {flake_ref:?}");
                if let Ok(flake_ref) = flake_ref {
                    let uri = if flake_ref.to_string().is_empty() {
                        uri.clone()
                    } else {
                        flake_ref.to_string()
                    };
                    if let Some(id) = flake_ref.id() {
                        change = Change::Add {
                            id: Some(id),
                            uri: Some(uri),
                            flake: !no_flake,
                        };
                    } else {
                        return Err(eyre::eyre!("Could not infer [ID] from flake reference.")
                            .with_note(|| format!("The provided uri: {uri}")));
                    }
                } else {
                    return Err(
                        eyre::eyre!("Could not infer [ID] from flake reference.")
                            .with_note(|| format!("The provided uri: {uri}"))
                            .suggestion(
                            "\nPlease specify an [ID] for this flake reference.\nIn the following form: `flake-edit add [ID] [uri]`\nIf you think the [ID] should have been able to be inferred, please open an issue.",
                        ),
                    );
                }
            }
        }
        cli::Command::Remove { id } => {
            if let Some(id) = id {
                change = Change::Remove {
                    id: id.to_owned().into(),
                };
            }
        }
        cli::Command::Completion { inputs: _, mode } => match mode {
            cli::CompletionMode::None => todo!(),
            cli::CompletionMode::Add => {
                let default_types = cache::default_types();
                for default in default_types {
                    println!("{}", default);
                }
                let cached_uris = cache::FeCache::default().get_or_init().list();
                for uri in cached_uris {
                    println!("{}", uri);
                }
                std::process::exit(0);
            }
        },
        Command::Pin { .. } | Command::Update { .. } | Command::List { .. } => {}
    }

    if let Ok(Some(resulting_change)) = editor.apply_change(change.clone()) {
        let root = rnix::Root::parse(&resulting_change.to_string());
        let errors = root.errors();
        if errors.is_empty() {
            tracing::info!("No errors in the changes.");
        } else {
            tracing::error!("There are errors in the changes:");
            eprintln!("There are errors in the changes:");
            for e in errors {
                tracing::error!("Error: {e}");
                tracing::error!("The changes will not be applied.");
            }
            eprintln!("{}", resulting_change);
            eprintln!("There were errors in the changes, the changes have not been applied.");
            std::process::exit(1);
        }

        // The changes are successful, so we can cache them
        if let Change::Add { id, uri, flake: _ } = change {
            let mut cache = cache::FeCache::default().get_or_init();
            cache.add_entry(id.unwrap(), uri.unwrap());
            cache
                .commit()
                .map_err(|e| eyre::eyre!("Could not write to cache file: {e}"))?;
        }

        app.apply_change_or_diff(&resulting_change, args.diff())?;
    } else if !args.list() && !args.update() && !args.pin() {
        if change.is_remove() {
            return Err(eyre::eyre!(
                "The input with id: {} could not be removed.",
                change.id().unwrap()
            )
            .suggestion("\nPlease check if an input with that [ID] exists in the flake.nix file.\nRun `flake-edit list --format simple` to see the current inputs by their id."));
        }
        println!("Nothing changed in the node.");
        println!("The following change could not be applied: \n{:?}", change);
        std::process::exit(1);
    }

    if let Command::List { format } = args.subcommand() {
        list_inputs(editor.list(), format);
    }
    if let Command::Update { id, init } = args.subcommand() {
        let inputs = editor.list();
        let mut buf = String::new();
        for input in inputs.values() {
            if !buf.is_empty() {
                buf.push('\n');
            }
            buf.push_str(input.id());
        }
        let mut updater = Updater::new(app.text().into(), inputs.clone());
        updater.update_all_inputs_to_latest_semver(id.clone(), *init);
        let change = updater.get_changes();
        app.apply_change_or_diff(&change, args.diff())?;
    }
    if let Command::Pin { id, rev } = args.subcommand() {
        let lock = FlakeLock::from_default_path().map_err(|_|
            eyre::eyre!(
                "The input with id: {} could not be pinned.",
                id,
            )
            .suggestion("\nPlease check if a `flake.lock` file exists.\nRun `nix flake lock` to initialize it.")
        )?;

        let target_rev = if let Some(rev) = rev {
            rev.to_string()
        } else {
            lock.get_rev_by_id(id)?
        };

        let inputs = editor.list();
        let mut buf = String::new();
        for input in inputs.values() {
            if !buf.is_empty() {
                buf.push('\n');
            }
            buf.push_str(input.id());
        }
        let mut updater = Updater::new(app.root().text().clone(), inputs.clone());

        updater.pin_input_to_ref(id, &target_rev);
        let change = updater.get_changes();
        app.apply_change_or_diff(&change, args.diff())?;
    }
    Ok(())
}
