use std::fmt::Display;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(author, version = CliArgs::unstable_version(), about, long_about = None)]
#[command(name = "flake-edit")]
#[command(next_line_help = true)]
/// Edit your flake inputs with ease
pub struct CliArgs {
    /// Location of the `flake.nix` file, that will be used.
    #[arg(long)]
    flake: Option<String>,
    /// Print a diff of the changes, will not write the changes to disk.
    #[arg(long, default_value_t = false)]
    diff: bool,
    /// Skip updating the lockfile after editing flake.nix.
    #[arg(long, default_value_t = false)]
    no_lock: bool,

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

    pub(crate) fn subcommand(&self) -> &Command {
        &self.subcommand
    }
    pub(crate) fn list(&self) -> bool {
        matches!(self.subcommand, Command::List { .. })
    }
    pub(crate) fn update(&self) -> bool {
        matches!(self.subcommand, Command::Update { .. })
    }
    pub(crate) fn pin(&self) -> bool {
        matches!(self.subcommand, Command::Pin { .. })
    }
    pub(crate) fn unpin(&self) -> bool {
        matches!(self.subcommand, Command::Unpin { .. })
    }
    pub(crate) fn change(&self) -> bool {
        matches!(self.subcommand, Command::Change { .. })
    }

    pub fn flake(&self) -> Option<&String> {
        self.flake.as_ref()
    }

    pub fn diff(&self) -> bool {
        self.diff
    }

    pub fn no_lock(&self) -> bool {
        self.no_lock
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
        /// Pin to a specific ref_or_rev
        ref_or_rev: Option<String>,
        /// The input itself is not a flake.
        #[arg(long, short)]
        no_flake: bool,
    },
    /// Remove a specific flake reference based on its id.
    #[clap(alias = "rm")]
    Remove { id: Option<String> },
    /// Change an existing flake reference's URI.
    #[clap(alias = "c")]
    #[command(arg_required_else_help = true)]
    Change {
        /// The name of an existing input attribute.
        id: Option<String>,
        /// The new URI for the input.
        uri: Option<String>,
        #[arg(long)]
        /// Pin to a specific ref_or_rev
        ref_or_rev: Option<String>,
    },
    /// List flake inputs
    #[clap(alias = "l")]
    List {
        #[arg(long, default_value_t = ListFormat::default())]
        format: ListFormat,
    },
    /// Update inputs to their latest specified release.
    #[clap(alias = "u")]
    Update {
        /// The id of an input attribute.
        /// If omitted will update all inputs.
        id: Option<String>,
        /// Whether the latest semver release of the remote should be used even thought the release
        /// itself isn't yet pinned to a specific release.
        #[arg(long)]
        init: bool,
    },
    /// Pin inputs to their current or a specified rev.
    #[clap(alias = "p")]
    Pin {
        /// The id of an input attribute.
        id: String,
        /// Optionally specify a rev for the inputs attribute.
        rev: Option<String>,
    },
    /// Unpin an input so it tracks the upstream default again.
    #[clap(alias = "up")]
    Unpin {
        /// The id of an input attribute.
        id: String,
    },
    #[clap(hide = true)]
    #[command(name = "completion")]
    /// Meant for shell completions.
    Completion {
        #[arg(long)]
        inputs: bool,
        mode: CompletionMode,
    },
}

#[derive(Debug, Clone, Default)]
/// Which command should be completed
pub(crate) enum CompletionMode {
    #[default]
    None,
    Add,
    Change,
}

impl From<String> for CompletionMode {
    fn from(value: String) -> Self {
        use CompletionMode::*;
        match value.to_lowercase().as_str() {
            "add" => Add,
            "change" => Change,
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) enum ListFormat {
    None,
    Simple,
    Toplevel,
    #[default]
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

impl Display for ListFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ListFormat::None => write!(f, ""),
            ListFormat::Simple => write!(f, "simple"),
            ListFormat::Toplevel => write!(f, "toplevel"),
            ListFormat::Detailed => write!(f, "detailed"),
            ListFormat::Raw => write!(f, "raw"),
            ListFormat::Json => write!(f, "json"),
        }
    }
}
