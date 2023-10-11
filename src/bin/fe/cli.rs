use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(author, version = CliArgs::unstable_version(), about, long_about = None)]
#[command(name = "fe")]
#[command(next_line_help = true)]
/// Edit your flake inputs with ease
pub(crate) struct CliArgs {
    // The `flake.nix` file that will be used
    #[arg(long)]
    flake: Option<String>,
    // The flake ref, or id that should be passed through to the nix command
    // By default will choose the local flake.
    flake_ref: Option<String>,
    /// Checks for potential errors in the setup
    #[arg(long)]
    health: bool,
    /// Pin to a specific ref_or_rev
    #[arg(long)]
    ref_or_rev: Option<String>,

    #[command(subcommand)]
    subcommand: Command,
}

#[allow(unused)]
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

    pub(crate) fn subcommand(&self) -> &Command {
        &self.subcommand
    }
    pub(crate) fn list(&self) -> bool {
        matches!(self.subcommand, Command::List { .. })
    }
}

#[derive(Subcommand, Debug)]
pub(crate) enum Command {
    /// Add a new flake reference.
    #[clap(alias = "a")]
    #[command(arg_required_else_help = true)]
    Add {
        /// The name of an input attribute.
        id: Option<String>,
        /// The uri that should be added to the input.
        // #[arg(last = true)]
        uri: Option<String>,
        #[arg(long)]
        ref_or_rev: Option<String>,
        /// Allow operations on uncommitted files.
        #[arg(long)]
        force: Option<String>,
    },
    /// Pin a specific flake reference based on its id.
    #[command(alias = "p", arg_required_else_help = true)]
    Pin { id: Option<String> },
    /// Change a specific flake reference based on its id.
    #[command(alias = "c", arg_required_else_help = true)]
    Change { id: Option<String> },
    /// Remove a specific flake reference based on its id.
    #[clap(alias = "rm")]
    Remove { id: Option<String> },
    /// List flake inputs
    #[clap(alias = "l")]
    List {
        #[arg(long, default_value = "detailed")]
        format: ListFormat,
    },
    #[clap(hide = true)]
    #[command(name = "completion")]
    /// Meant for shell completions.
    Completion {
        #[arg(long)]
        inputs: bool,
    },
}

#[derive(Debug, Clone)]
pub(crate) enum Parameter {}

#[derive(Debug, Clone, Default)]
pub(crate) enum ListFormat {
    None,
    #[default]
    Simple,
    Toplevel,
    Detailed,
    Raw,
    Json,
}

impl From<String> for ListFormat {
    fn from(value: String) -> Self {
        use ListFormat::*;
        match value.to_lowercase().as_str() {
            "detailed" => Detailed,
            "simple" => Simple,
            "toplevel" => Toplevel,
            "raw" => Raw,
            "json" => Json,
            _ => None,
        }
    }
}
