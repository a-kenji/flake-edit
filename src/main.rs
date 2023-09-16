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
mod log;

fn main() -> anyhow::Result<()> {
    let args = CliArgs::parse();
    log::init()?;
    tracing::debug!("Cli args: {args:?}");

    // let inputs = r#"{ inputs = { nixpkgs.url = "github:nixos/nixpkgs";};}"#;
    // let inputs = r#"{inputs = { nixpkgs.url = "github:nixos/nixpkgs"; crane.url = "github:nix-community/crane"; };}"#;
    // let inputs = r#"{ inputs.nixpkgs.url = github:nixos/nixpkgs; inputs.crane.url = "github:ivpetkov/crane";}"#;

    let inputs = r#"{
      description = "Manage your flake inputs comfortably.";

      inputs = {
        nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
        #flake-utils.url = "github:numtide/flake-utils";
        rust-overlay = {
          url = "github:oxalica/rust-overlay";
          inputs.nixpkgs.follows = "nixpkgs";
          # inputs.flake-utils.follows = "flake-utils";
        };
        crane = {
          url = "github:ipetkov/crane";
          # inputs.nixpkgs.follows = "nixpkgs";
          # inputs.rust-overlay.follows = "rust-overlay";
          # inputs.flake-utils.follows = "flake-utils";
        };
        vmsh.url = "github:mic92/vmsh";
      };
      }
    "#;

    let app = FlakeAdd::init()?;

    let (node, _errors) = rnix::parser::parse(Tokenizer::new(inputs));
    // let (node, _errors) = rnix::parser::parse(Tokenizer::new(&app.root.text.to_string()));

    let mut state = flake_add::State::default();

    if let Some(command) = args.subcommand() {
        match command {
            cli::Command::Add { add, ref_or_rev } => {
                let change = flake_add::Change::Change {
                    id: add.clone(),
                    ref_or_rev: ref_or_rev.clone(),
                };
                state.add_change(change);
            }
            cli::Command::Pin { .. } => todo!(),
            cli::Command::Remove { .. } => todo!(),
        }
    }

    // let change = flake_add::Change::Change {
    //     id: Some("crane".into()),
    //     ref_or_rev: Some("test".to_owned()),
    // };
    // state.add_change(change);
    state.walk_attr_set(&node);
    // let stream = &app.root.text.to_string();
    // state.walk_expr_set(stream);

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
