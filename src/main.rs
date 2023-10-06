//! Will look in the following manner for a `flake.nix` file:
//!     - In the cwd
//!     - In the directory upwards to `git_root`
//!
use std::fs::File;
use std::io;

use crate::cli::CliArgs;
use clap::Parser;
use flake_add::diff::Diff;
use flake_add::walk::Walker;
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

    // nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    let inputs = r#"{
      description = "Manage your flake inputs comfortably.";

      inputs = {
        flake-utils.url = "github:numtide/flake-utils";
        flake-utils.flake = false;
        rust-overlay = {
          url = "github:oxalica/rust-overlay";
          inputs.flake-utils.follows = "flake-utils";
        };
      };
      }
    "#;

    // let inputs = r#"{
    //   description = "Manage your flake inputs comfortably.";
    //
    //   inputs.nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    //   inputs.flake-utils.url = "github:numtide/flake-utils";
    //   inputs.flake-utils.flake = false;
    //   inputs.rust-overlay = {
    //       url = "github:oxalica/rust-overlay";
    //       inputs.flake-utils.follows = "flake-utils";
    //     };
    //   };
    //   }
    // "#;

    // let inputs = r#"{
    //   description = "Manage your flake inputs comfortably.";
    //
    //   inputs = {
    //     nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    //     flake-utils.url = "github:numtide/flake-utils";
    //     flake-utils.flake = false;
    //     rust-overlay = {
    //       url = "github:oxalica/rust-overlay";
    //       #inputs.nixpkgs.follows = "nixpkgs";
    //       inputs.flake-utils.follows = "flake-utils";
    //     };
    //     #crane = {
    //       #url = "github:ipetkov/crane";
    //       # inputs.nixpkgs.follows = "nixpkgs";
    //       # inputs.rust-overlay.follows = "rust-overlay";
    //       # inputs.flake-utils.follows = "flake-utils";
    //     #};
    //     #vmsh.url = "github:mic92/vmsh";
    //   };
    //   }
    // "#;

    let app = FlakeAdd::init()?;

    // let (node, errors) = rnix::parser::parse(Tokenizer::new(inputs));
    let (node, errors) = rnix::parser::parse(Tokenizer::new(&app.root.text.to_string()));
    if !errors.is_empty() {
        println!("There are errors in the root document.");
    }

    // let mut walker = Walker::new(inputs).unwrap();
    let text = app.root.text.to_string();
    let mut walker = Walker::new(&text).unwrap();

    let mut state = flake_add::State::default();

    match args.subcommand() {
        cli::Command::Add {
            uri,
            ref_or_rev: _,
            id,
        } => {
            let change = flake_add::Change::Add {
                id: id.clone(),
                uri: uri.clone(),
            };
            walker.changes.push(change);
        }
        cli::Command::Pin { .. } => todo!(),
        cli::Command::Remove { id } => {
            if let Some(id) = id {
                let change = flake_add::Change::Remove { id: id.clone() };
                walker.changes.push(change);
            }
        }
        cli::Command::List { .. } => {}
    }

    if let Some(change) = walker.walk_toplevel() {
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
    } else if args.list() {
        println!("{:#?}", walker.inputs);
    } else {
        println!("Nothing changed in the node.");
        for change in walker.changes {
            println!("The following change could not be applied: \n{:?}", change);
        }
    }

    // let change = flake_add::Change::Change {
    //     id: Some("crane".into()),
    //     ref_or_rev: Some("test".to_owned()),
    // };
    // state.add_change(change);

    // state.walk_attr_set(&node);

    // let stream = &app.root.text.to_string();
    // state.walk_expr_set(stream);

    if args.list() {
        println!("{:#?}", state.inputs);
    } else {
        // println!("Inputs:");
        // println!("State: {:#?}", state);
    }

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
