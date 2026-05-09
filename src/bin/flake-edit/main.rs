use std::process::ExitCode;

use clap::Parser;
use flake_edit::cli::CliArgs;

mod log;
mod render;

fn main() -> ExitCode {
    let args = CliArgs::parse();

    log::init().ok();
    tracing::debug!("Cli args: {args:?}");

    match flake_edit::app::run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            render::report(&err);
            ExitCode::FAILURE
        }
    }
}
