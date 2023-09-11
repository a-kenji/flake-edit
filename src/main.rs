//! Will look in the following manner for a `flake.nix` file:
//!     - In the cwd
//!     - In the directory upwards to `git_root`
//!
use std::fs::File;
use std::io;

use crate::cli::CliArgs;
use clap::Parser;
use rnix::tokenizer::Tokenizer;
use ropey::Rope;

mod cli;
mod error;

fn main() -> anyhow::Result<()> {
    let args = CliArgs::parse();
    println!("{:?}", args);
    // let inputs = r#"{ inputs = { nixpkgs.url = "github:nixos/nixpkgs";};}"#;
    let inputs = r#"{inputs = { nixpkgs.url = "github:nixos/nixpkgs"; crane.url = "github:nix-community/crane"; };}"#;
    // let inputs = r#"{ inputs = { nixpkgs.url = github:nixos/nixpkgs;};}"#;

    let app = FlakeAdd::init()?;

    // let (node, _errors) = rnix::parser::parse(Tokenizer::new(inputs));
    let (node, _errors) = rnix::parser::parse(Tokenizer::new(&app.root.text.to_string()));

    let mut state = flake_add::State::default();

    match args.command() {
        Some(command) => match command {
            cli::Command::Add { add } => {
                let change = flake_add::Change::Add { input: add.clone() };
                state.add_change(change);
            }
            cli::Command::Pin { pin } => todo!(),
            cli::Command::Remove { remove } => todo!(),
        },
        None => {}
    }

    state.walk_attr_set(&node);

    println!("State: {:#?}", state);

    Ok(())
}

#[derive(Debug, Default)]
pub struct FlakeAdd {
    root: FlakeBuf,
    _lock: Option<FlakeBuf>,
}

impl FlakeAdd {
    pub fn init() -> io::Result<Self> {
        let root = FlakeBuf::from_path("flake.nix")?;
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
    fn from_path(path: &str) -> io::Result<Self> {
        let text = Rope::from_reader(&mut io::BufReader::new(File::open(path)?))?;
        Ok(Self {
            text,
            path: path.to_string(),
            dirty: false,
        })
    }
}
