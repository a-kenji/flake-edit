//! Will look in the following manner for a `flake.nix` file:
//!     - In the cwd
//!     - In the directory upwards to `git_root`
//!
use crate::cli::CliArgs;
use clap::Parser;

mod cli;

fn main() -> Result<(), ()> {
    let args = CliArgs::parse();

    Ok(())
}
