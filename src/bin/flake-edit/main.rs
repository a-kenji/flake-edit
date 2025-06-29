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
use flake_edit::error::FlakeEditError;
use flake_edit::lock::FlakeLock;
use flake_edit::update::Updater;
use list::list_inputs;
use nix_uri::urls::UrlWrapper;
use nix_uri::{FlakeRef, NixUriResult};
use std::io::{self, Write};

mod app;
mod cache;
mod cli;
mod error;
mod list;
mod log;
mod root;

fn prompt_version_selection(
    input_id: &str,
    active_versions: &[String],
    commented_versions: &[String],
) -> eyre::Result<usize> {
    println!("Multiple versions of '{}' found:", input_id);
    println!();

    let mut options = Vec::new();

    for (i, version) in commented_versions.iter().enumerate() {
        println!("  {}: {} (commented)", i + 1, version);
        options.push((i + 1, version.clone(), false))
    }

    if !active_versions.is_empty() {
        let active_idx = commented_versions.len() + 1;
        println!(
            "  {}: {} (currently active)",
            active_idx, active_versions[0]
        );
        options.push((active_idx, active_versions[0].clone(), true));
    }

    println!();
    print!("Select which version to activate (1-{}): ", options.len());
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    let choice: usize = input
        .trim()
        .parse()
        .map_err(|_| eyre::eyre!("Invalid selection. Please enter a number."))?;

    if choice < 1 || choice > options.len() {
        return Err(eyre::eyre!(
            "Invalid selection. Please enter a number between 1 and {}.",
            options.len()
        ));
    }
    // Convert to 0-based index
    Ok(choice - 1)
}

fn diagnose_toggle_error(change: &Change, app: &FlakeEdit) -> eyre::Result<()> {
    if let Change::Toggle { id } = change {
        let flake_text = app.text();
        let lines: Vec<&str> = flake_text.lines().collect();

        // Use the same logic as in walker to determine the error
        use std::collections::HashSet;
        let mut toggleable_inputs = HashSet::new();

        // Find all inputs that have both commented and uncommented versions
        for line in &lines {
            let trimmed = line.trim();

            // Check for active .url lines
            if let Some(pos) = trimmed.find(".url") {
                if !trimmed.starts_with('#') {
                    let input_id = &trimmed[..pos];
                    // Check if there's a commented version of this input
                    let commented_pattern = format!("# {}.url", input_id);
                    if lines
                        .iter()
                        .any(|l| l.trim().starts_with(&commented_pattern))
                    {
                        toggleable_inputs.insert(input_id.to_string());
                    }
                }
            }

            // Check for commented .url lines
            if trimmed.starts_with("# ") {
                let uncommented = trimmed.strip_prefix("# ").unwrap_or(trimmed);
                if let Some(pos) = uncommented.find(".url") {
                    let input_id = &uncommented[..pos];
                    // Check if there's an active version of this input
                    let active_pattern = format!("{}.url", input_id);
                    if lines.iter().any(|l| {
                        l.trim().starts_with(&active_pattern) && !l.trim().starts_with('#')
                    }) {
                        toggleable_inputs.insert(input_id.to_string());
                    }
                }
            }
        }

        if let Some(specified_id) = id {
            // User specified an id but it wasn't toggleable
            // check if it has multiple versions
            let editor = app.create_editor().unwrap();
            let (active_versions, commented_versions) =
                editor.walker().get_input_versions(&lines, specified_id);

            if (active_versions.len() == 1 && commented_versions.len() > 1)
                || (active_versions.is_empty() && commented_versions.len() > 1)
            {
                let mut options = Vec::new();
                for (i, version) in commented_versions.iter().enumerate() {
                    options.push(format!("  {}: {}", i + 1, version));
                }
                if !active_versions.is_empty() {
                    options.push(format!(
                        "  {}: {} (currently active)",
                        options.len() + 1,
                        active_versions[0]
                    ));
                }
                let options_text = options.join("\n");
                return Err(eyre::eyre!(FlakeEditError::MultipleVersionsNeedSelection(
                    specified_id.clone(),
                    options_text
                )));
            } else {
                return Err(eyre::eyre!(FlakeEditError::NoToggleableVersions(
                    specified_id.clone()
                )));
            }
        } else {
            match toggleable_inputs.len() {
                0 => return Err(eyre::eyre!(FlakeEditError::NoToggleableInputs)),
                1 => {
                    return Err(eyre::eyre!(
                        "Internal error: auto-detection should have succeeded"
                    ));
                }
                _ => {
                    let inputs_list = {
                        let mut sorted: Vec<_> = toggleable_inputs.into_iter().collect();
                        sorted.sort();
                        sorted.join(", ")
                    };
                    return Err(eyre::eyre!(FlakeEditError::MultipleToggleableInputs(
                        inputs_list
                    )));
                }
            }
        }
    }

    Err(eyre::eyre!("Unknown toggle error"))
}

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
        Command::Toggle { id } => {
            let input_id = if let Some(input_id) = id {
                input_id.clone()
            } else {
                // Auto-detect the input ID
                let flake_text = app.text();
                let lines: Vec<&str> = flake_text.lines().collect();

                use std::collections::HashSet;
                let mut toggleable_inputs = HashSet::new();

                for line in &lines {
                    let trimmed = line.trim();

                    // Check for active .url lines
                    if let Some(pos) = trimmed.find(".url") {
                        if !trimmed.starts_with('#') {
                            let input_id = &trimmed[..pos];
                            let commented_pattern = format!("# {}.url", input_id);
                            if lines
                                .iter()
                                .any(|l| l.trim().starts_with(&commented_pattern))
                            {
                                toggleable_inputs.insert(input_id.to_string());
                            }
                        }
                    }

                    // Check for commented .url lines
                    if trimmed.starts_with("# ") {
                        let uncommented = trimmed.strip_prefix("# ").unwrap_or(trimmed);
                        if let Some(pos) = uncommented.find(".url") {
                            let input_id = &uncommented[..pos];
                            let active_pattern = format!("{}.url", input_id);
                            if lines.iter().any(|l| {
                                l.trim().starts_with(&active_pattern) && !l.trim().starts_with('#')
                            }) {
                                toggleable_inputs.insert(input_id.to_string());
                            }
                        }
                    }
                }

                match toggleable_inputs.len() {
                    0 => return Err(eyre::eyre!(FlakeEditError::NoToggleableInputs)),
                    1 => toggleable_inputs.into_iter().next().unwrap(),
                    _ => {
                        let inputs_list = {
                            let mut sorted: Vec<_> = toggleable_inputs.into_iter().collect();
                            sorted.sort();
                            sorted.join(", ")
                        };
                        return Err(eyre::eyre!(FlakeEditError::MultipleToggleableInputs(
                            inputs_list
                        )));
                    }
                }
            };

            // Now check if the input ID has multiple versions
            let editor = app.create_editor()?;
            let flake_text = app.text();
            let lines: Vec<&str> = flake_text.lines().collect();
            let (active_versions, commented_versions) =
                editor.walker().get_input_versions(&lines, &input_id);

            if (active_versions.len() == 1 && commented_versions.len() > 1)
                || (active_versions.is_empty() && commented_versions.len() > 1)
            {
                if args.non_interactive() {
                    let mut options = Vec::new();
                    for (i, version) in commented_versions.iter().enumerate() {
                        options.push(format!("  {}: {}", i + 1, version));
                    }
                    if !active_versions.is_empty() {
                        options.push(format!(
                            "  {}: {} (currently active)",
                            options.len() + 1,
                            active_versions[0]
                        ));
                    }
                    let options_text = options.join("\n");
                    return Err(eyre::eyre!(FlakeEditError::MultipleVersionsNeedSelection(
                        input_id.clone(),
                        options_text
                    )));
                } else {
                    // Interactive mode - prompt user
                    let selection =
                        prompt_version_selection(&input_id, &active_versions, &commented_versions)?;

                    // Determine which version to activate based on selection
                    let target_url = if selection < commented_versions.len() {
                        // User selected a commented version to activate
                        commented_versions[selection].clone()
                    } else {
                        // User selected the currently active version (essentially a no-op, but we could comment it out)
                        return Err(eyre::eyre!(
                            "Cannot activate the currently active version. It's already active."
                        ));
                    };

                    // Create a targeted toggle change
                    change = Change::ToggleToVersion {
                        id: input_id.clone(),
                        target_url,
                    };
                }
            } else {
                change = Change::Toggle {
                    id: Some(input_id.clone()),
                };
            }
        }
    }

    match editor.apply_change(change.clone()) {
        Ok(Some(resulting_change)) => {
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

            app.apply_change_or_diff(&resulting_change, args.diff(), args.no_lock())?;
        }
        Err(e) => {
            return Err(eyre::eyre!(e.to_string()));
        }
        Ok(None) => {
            if !args.list() && !args.update() && !args.pin() {
                if change.is_remove() {
                    return Err(eyre::eyre!(
                "The input with id: {} could not be removed.",
                change.id().unwrap()
            )
            .suggestion("\nPlease check if an input with that [ID] exists in the flake.nix file.\nRun `flake-edit list --format simple` to see the current inputs by their id."));
                } else if change.is_toggle() {
                    // Handle toggle-specific errors
                    return diagnose_toggle_error(&change, &app);
                }
                println!("Nothing changed in the node.");
                println!("The following change could not be applied: \n{:?}", change);
                std::process::exit(1);
            }
        }
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
        app.apply_change_or_diff(&change, args.diff(), args.no_lock())?;
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
        app.apply_change_or_diff(&change, args.diff(), args.no_lock())?;
    }
    Ok(())
}
