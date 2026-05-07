//! `flake-edit config`: surface configuration without mutating the
//! flake.
//!
//! `--print-default` writes the embedded default
//! [`DEFAULT_CONFIG_TOML`] to stdout. `--path` reports the lookup
//! locations for the project and user config files. With neither
//! flag the subcommand is a no-op.

use crate::config::{Config, DEFAULT_CONFIG_TOML};

use super::Result;

pub fn config(print_default: bool, path: bool) -> Result<()> {
    if print_default {
        print!("{}", DEFAULT_CONFIG_TOML);
        return Ok(());
    }

    if path {
        let project_path = Config::project_config_path();
        let user_path = Config::user_config_path();

        if let Some(path) = &project_path {
            println!("Project config: {}", path.display());
        }
        if let Some(path) = &user_path {
            println!("User config: {}", path.display());
        }

        if project_path.is_none() && user_path.is_none() {
            if let Some(user_dir) = Config::user_config_dir() {
                println!("No config found. Create one at:");
                println!("  Project: flake-edit.toml (in current directory)");
                println!("  User:    {}/config.toml", user_dir.display());
            } else {
                println!("No config found. Create flake-edit.toml in current directory.");
            }
        }
        return Ok(());
    }

    Ok(())
}
