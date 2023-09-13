use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(author, version = CliArgs::unstable_version(), about, long_about = None)]
#[command(next_line_help = true)]
pub(crate) struct CliArgs {
    // The flake ref, or id that should be passed through to the nix command
    // By default will choose the local flake.
    flake_ref: Option<String>,
    /// Checks for potential errors in the setup
    #[arg(long)]
    health: bool,
    /// Pin to a specific ref_or_rev
    #[arg(long)]
    ref_or_rev: Option<String>,
    // /// Set a flake parameter for a flake_ref
    // #[arg(long)]
    // param: Option<Parameter>,
    #[command(subcommand)]
    command: Option<Command>,
}

impl CliArgs {
    /// Surface current version together with the current git revision and date, if available
    fn unstable_version() -> &'static str {
        const VERSION: &str = env!("CARGO_PKG_VERSION");
        let date = option_env!("GIT_DATE").unwrap_or("no_date");
        let rev = option_env!("GIT_REV").unwrap_or("no_rev");
        // This is a memory leak, only use sparingly.
        Box::leak(format!("{VERSION} - {date} - {rev}").into_boxed_str())
    }

    pub(crate) fn get_flake_ref(&self) -> Option<String> {
        self.flake_ref.clone()
    }

    pub(crate) fn command(&self) -> Option<&Command> {
        self.command.as_ref()
    }
}

#[derive(Subcommand, Debug)]
pub(crate) enum Command {
    /// Add a new flake reference.
    #[clap(alias = "a")]
    Add {
        add: Option<String>,
        #[arg(long)]
        ref_or_rev: Option<String>,
    },
    /// Pin a specific flake reference based on its id.
    Pin { pin: Option<String> },
    /// Remove a specific flake reference, based on its id.
    Remove { remove: Option<String> },
}

#[derive(Debug, Clone)]
pub(crate) enum Parameter {}
