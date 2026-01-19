use std::fmt::Display;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(author, version = CliArgs::unstable_version(), about, long_about = None)]
#[command(name = "flake-edit")]
#[command(next_line_help = true)]
/// Edit your flake inputs with ease
pub struct CliArgs {
    /// Location of the `flake.nix` file, that will be used.
    /// Defaults to `flake.nix` in the current directory.
    #[arg(long)]
    flake: Option<String>,
    /// Location of the `flake.lock` file.
    /// Defaults to `flake.lock` in the current directory.
    #[arg(long)]
    lock_file: Option<String>,
    /// Print a diff of the changes, will not write the changes to disk.
    #[arg(long, default_value_t = false)]
    diff: bool,
    /// Skip updating the lockfile after editing flake.nix.
    #[arg(long, default_value_t = false)]
    no_lock: bool,
    /// Disable interactive prompts.
    #[arg(long, default_value_t = false)]
    non_interactive: bool,
    /// Disable reading from and writing to the completion cache.
    #[arg(long, default_value_t = false)]
    no_cache: bool,
    /// Path to a custom cache file.
    #[arg(long)]
    cache: Option<String>,
    /// Path to a custom configuration file.
    #[arg(long)]
    config: Option<String>,

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

    pub fn subcommand(&self) -> &Command {
        &self.subcommand
    }
    pub fn list(&self) -> bool {
        matches!(self.subcommand, Command::List { .. })
    }
    pub fn update(&self) -> bool {
        matches!(self.subcommand, Command::Update { .. })
    }
    pub fn pin(&self) -> bool {
        matches!(self.subcommand, Command::Pin { .. })
    }
    pub fn unpin(&self) -> bool {
        matches!(self.subcommand, Command::Unpin { .. })
    }
    pub fn change(&self) -> bool {
        matches!(self.subcommand, Command::Change { .. })
    }
    pub fn follow(&self) -> bool {
        matches!(self.subcommand, Command::Follow { .. })
    }

    pub fn flake(&self) -> Option<&String> {
        self.flake.as_ref()
    }

    pub fn lock_file(&self) -> Option<&String> {
        self.lock_file.as_ref()
    }

    pub fn diff(&self) -> bool {
        self.diff
    }

    pub fn no_lock(&self) -> bool {
        self.no_lock
    }

    pub fn non_interactive(&self) -> bool {
        self.non_interactive
    }

    pub fn no_cache(&self) -> bool {
        self.no_cache
    }

    pub fn cache(&self) -> Option<&String> {
        self.cache.as_ref()
    }

    pub fn config(&self) -> Option<&String> {
        self.config.as_ref()
    }
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Add a new flake reference.
    #[clap(alias = "a")]
    Add {
        /// The name of an input attribute.
        id: Option<String>,
        /// The uri that should be added to the input.
        uri: Option<String>,
        #[arg(long)]
        /// Pin to a specific ref_or_rev
        ref_or_rev: Option<String>,
        /// The input itself is not a flake.
        #[arg(long, short)]
        no_flake: bool,
        /// Use shallow clone for the input.
        #[arg(long, short)]
        shallow: bool,
    },
    /// Remove a specific flake reference based on its id.
    #[clap(alias = "rm")]
    Remove { id: Option<String> },
    /// Change an existing flake reference's URI.
    #[clap(alias = "c")]
    Change {
        /// The name of an existing input attribute.
        id: Option<String>,
        /// The new URI for the input.
        uri: Option<String>,
        #[arg(long)]
        /// Pin to a specific ref_or_rev
        ref_or_rev: Option<String>,
        /// Use shallow clone for the input.
        #[arg(long, short)]
        shallow: bool,
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
        id: Option<String>,
        /// Optionally specify a rev for the inputs attribute.
        rev: Option<String>,
    },
    /// Unpin an input so it tracks the upstream default again.
    #[clap(alias = "up")]
    Unpin {
        /// The id of an input attribute.
        id: Option<String>,
    },
    /// Automatically add and remove follows declarations.
    ///
    /// Analyzes the flake.lock to find nested inputs that match top-level inputs,
    /// then adds appropriate follows declarations and removes stale ones.
    ///
    /// With file paths, processes multiple flakes in batch.
    /// For every `flake.nix` file passed in it will assume a
    /// `flake.lock` file exists in the same directory.
    #[clap(alias = "f")]
    Follow {
        /// Flake.nix paths to process. If empty, runs on current directory.
        #[arg(trailing_var_arg = true, num_args = 0..)]
        paths: Vec<std::path::PathBuf>,
    },
    /// Manually add a single follows declaration.
    ///
    /// Example: `flake-edit add-follow rust-overlay.nixpkgs nixpkgs`
    ///
    /// This creates: `rust-overlay.inputs.nixpkgs.follows = "nixpkgs";`
    ///
    /// Without arguments, starts an interactive selection.
    #[clap(alias = "af")]
    AddFollow {
        /// The input path in dot notation (e.g., "rust-overlay.nixpkgs" means
        /// the nixpkgs input of rust-overlay).
        input: Option<String>,
        /// The target input to follow (e.g., "nixpkgs").
        target: Option<String>,
    },
    #[clap(hide = true)]
    #[command(name = "completion")]
    /// Meant for shell completions.
    Completion {
        #[arg(long)]
        inputs: bool,
        mode: CompletionMode,
    },
    /// Manage flake-edit configuration.
    #[clap(alias = "cfg", arg_required_else_help = true)]
    Config {
        /// Output the default configuration to stdout.
        #[arg(long)]
        print_default: bool,
        /// Show where configuration would be loaded from.
        #[arg(long)]
        path: bool,
    },
}

#[derive(Debug, Clone, Default)]
/// Which command should be completed
pub enum CompletionMode {
    #[default]
    None,
    Add,
    Change,
    Follow,
}

impl From<String> for CompletionMode {
    fn from(value: String) -> Self {
        use CompletionMode::*;
        match value.to_lowercase().as_str() {
            "add" => Add,
            "change" => Change,
            "follow" => Follow,
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub enum ListFormat {
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
