//! Will look in the following manner for a `flake.nix` file:
//!     - In the cwd
//!     - In the directory upwards to `git_root`
//!
use std::fs::File;
use std::io;
use std::path::PathBuf;

use crate::cli::CliArgs;
use crate::cli::Command;
use clap::Parser;
use flake_edit::change::Change;
use flake_edit::diff::Diff;
use flake_edit::input::Follows;
use flake_edit::walk::Walker;
use nix_uri::urls::UrlWrapper;
use nix_uri::{FlakeRef, NixUriResult};
use rnix::tokenizer::Tokenizer;
use ropey::Rope;

use self::error::FeError;

mod cli;
mod error;
mod log;
mod root;

fn main() -> anyhow::Result<()> {
    let args = CliArgs::parse();
    log::init()?;
    tracing::debug!("Cli args: {args:?}");

    let app = FlakeAdd::init()?;

    let (_node, errors) = rnix::parser::parse(Tokenizer::new(&app.root.text.to_string()));
    if !errors.is_empty() {
        println!("There are errors in the root document.");
    }

    // let mut walker = Walker::new(inputs).unwrap();
    let text = app.root.text.to_string();
    let mut walker = Walker::new(&text);

    // let mut state = flake_edit::State::default();

    match args.subcommand() {
        cli::Command::Add {
            uri,
            ref_or_rev: _,
            id,
            force: _,
        } => {
            if id.is_some() && uri.is_some() {
                let change = Change::Add {
                    id: id.clone(),
                    uri: uri.clone(),
                };
                walker.changes.push(change);
            } else if let Some(uri) = id {
                let flake_ref: NixUriResult<FlakeRef> = UrlWrapper::convert_or_parse(uri);
                if let Ok(flake_ref) = flake_ref {
                    let uri = if flake_ref.to_string().is_empty() {
                        uri.clone()
                    } else {
                        flake_ref.to_string()
                    };
                    if let Some(id) = flake_ref.id() {
                        let change = Change::Add {
                            id: Some(id),
                            uri: Some(uri),
                        };
                        walker.changes.push(change);
                    } else {
                        println!("Please specify an [ID] for this flake reference.")
                    }
                } else {
                    println!("Please specify an [ID] for this flake reference.")
                }
            }
        }
        cli::Command::Pin { .. } => todo!(),
        cli::Command::Remove { id } => {
            if let Some(id) = id {
                let change = Change::Remove { id: id.clone() };
                walker.changes.push(change);
            }
        }
        cli::Command::List { .. } => {}
        cli::Command::Change { id: _ } => todo!(),
        cli::Command::Completion { inputs: _ } => todo!(),
    }

    if let Some(change) = walker.walk() {
        let root = rnix::Root::parse(&change.to_string());
        let errors = root.errors();
        if errors.is_empty() {
            println!("No errors in the changes.");
        } else {
            println!("There are errors in the changes.");
        }
        // println!("Original Node: \n{}\n", walker.root);
        // println!("Changed Node: \n{}\n", change);
        let old = walker.root.to_string();
        let new = change.to_string();
        let diff = Diff::new(&old, &new);
        diff.compare();
    } else if !args.list() {
        println!("Nothing changed in the node.");
        for change in walker.changes {
            println!("The following change could not be applied: \n{:?}", change);
        }
    }

    if let Command::List { format } = args.subcommand() {
        match format {
            cli::ListFormat::Simple => {
                let inputs = walker.inputs;
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
                let inputs = walker.inputs;
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
                println!("{:#?}", walker.inputs);
            }
            cli::ListFormat::Json => {
                let json = serde_json::to_string(&walker.inputs).unwrap();
                println!("{json}");
            }
            cli::ListFormat::None => todo!(),
            cli::ListFormat::Toplevel => {
                let inputs = walker.inputs;
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

#[derive(Debug, Default)]
pub struct FlakeAdd {
    root: FlakeBuf,
    _lock: Option<FlakeBuf>,
}

impl FlakeAdd {
    const FLAKE: &str = "flake.nix";
    pub fn init() -> Result<Self, FeError> {
        let path = PathBuf::from(Self::FLAKE);
        let binding = root::Root::from_path(path)?;
        let root = binding.path();
        let root = FlakeBuf::from_path(root.to_path_buf())?;
        Ok(Self { root, _lock: None })
    }
}

#[derive(Debug, Default)]
pub struct FlakeBuf {
    text: Rope,
    path: String,
    dirty: bool,
}

impl FlakeBuf {
    fn from_path(path: PathBuf) -> io::Result<Self> {
        let text = Rope::from_reader(&mut io::BufReader::new(File::open(&path)?))?;
        let path = format!("{:?}", path);
        Ok(Self {
            text,
            path,
            dirty: false,
        })
    }
}
