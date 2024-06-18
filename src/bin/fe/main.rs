//! Will look in the following manner for a `flake.nix` file:
//!     - In the cwd
//!     - In the directory upwards to `git_root`
//!
use crate::app::FlakeEdit;
use crate::cli::CliArgs;
use crate::cli::Command;
use clap::Parser;
use color_eyre::eyre;
use color_eyre::Section;
use flake_edit::change::Change;
use flake_edit::diff::Diff;
use flake_edit::edit;
use flake_edit::input::Follows;
use nix_uri::urls::UrlWrapper;
use nix_uri::{FlakeRef, NixUriResult};
use rnix::tokenizer::Tokenizer;

mod app;
mod cache;
mod cli;
mod error;
mod log;
mod root;

fn main() -> eyre::Result<()> {
    let args = CliArgs::parse();
    color_eyre::install()?;
    log::init().ok();
    tracing::debug!("Cli args: {args:?}");

    let app = FlakeEdit::init(&args)?;

    let (_node, errors) = rnix::parser::parse(Tokenizer::new(&app.root().text().to_string()));
    if !errors.is_empty() {
        tracing::error!("There are errors in the root document.");
    }

    let text = app.root().text().to_string();
    let mut editor = edit::FlakeEdit::from(&text)?;
    let mut change = Change::None;

    match args.subcommand() {
        cli::Command::Add {
            uri,
            ref_or_rev: _,
            id,
            force: _,
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
                            "\nPlease specify an [ID] for this flake reference.\nIn the following form: `fe add [ID] [uri]`\nIf you think the [ID] should have been able to be inferred, please open an issue.",
                        ),
                    );
                }
            }
        }
        cli::Command::Pin { .. } => todo!(),
        cli::Command::Remove { id } => {
            if let Some(id) = id {
                change = Change::Remove {
                    id: id.to_owned().into(),
                };
            }
        }
        cli::Command::List { .. } => {}
        cli::Command::Change { id, uri } => {
            if let Some(id) = id {
                change = Change::Change {
                    id: id.to_owned().into(),
                    ref_or_rev: None,
                    uri: uri.clone(),
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

        if args.diff() {
            let old = text;
            let new = resulting_change;
            let diff = Diff::new(&old, &new);
            diff.compare();
            // Write the changes
        } else if args.apply() {
            app.root.apply(&resulting_change)?;
        }
    } else if !args.list() {
        if change.is_remove() {
            return Err(eyre::eyre!(
                "The input with id: {} could not be removed.",
                change.id().unwrap()
            )
            .suggestion("\nPlease check if an input with that [ID] exists in the flake.nix file.\nRun `fe list --format simple` to see the current inputs by their id."));
        }
        println!("Nothing changed in the node.");
        println!("The following change could not be applied: \n{:?}", change);
        std::process::exit(1);
    }

    if let Command::List { format } = args.subcommand() {
        match format {
            cli::ListFormat::Simple => {
                let inputs = editor.list();
                let mut buf = String::new();
                for input in inputs.values() {
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
            cli::ListFormat::Detailed => {
                let inputs = editor.list();
                let mut buf = String::new();
                for input in inputs.values() {
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
            cli::ListFormat::Raw => {
                println!("{:#?}", editor.list());
            }
            cli::ListFormat::Json => {
                let json = serde_json::to_string(editor.list()).unwrap();
                println!("{json}");
            }
            cli::ListFormat::None => todo!(),
            cli::ListFormat::Toplevel => {
                let inputs = editor.list();
                let mut buf = String::new();
                for input in inputs.keys() {
                    if !buf.is_empty() {
                        buf.push('\n');
                    }
                    buf.push_str(&input.to_string());
                }
                println!("{buf}");
            }
        }
    }
    Ok(())
}
