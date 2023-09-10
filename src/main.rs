//! Will look in the following manner for a `flake.nix` file:
//!     - In the cwd
//!     - In the directory upwards to `git_root`
//!
use crate::cli::CliArgs;
use clap::Parser;
use rnix::tokenizer::Tokenizer;

mod cli;

fn main() -> Result<(), ()> {
    let _args = CliArgs::parse();
    let inputs = r#"{ inputs = { nixpkgs.url = "github:nixos/nixpkgs";};}"#;
    let (node, _errors) = rnix::parser::parse(Tokenizer::new(inputs));

    let mut state = flake_add::State::default();

    state.walk_attr_set(&node);

    println!("State: {:#?}", state);

    Ok(())
}
