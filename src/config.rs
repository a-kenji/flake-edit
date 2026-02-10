use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Default configuration embedded in the binary.
pub const DEFAULT_CONFIG_TOML: &str = include_str!("assets/config.toml");

/// Error type for configuration loading failures.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Failed to read config file '{path}': {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("Failed to parse config file '{path}':\n{source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
}

/// Filenames to search for project-level configuration.
const CONFIG_FILENAMES: &[&str] = &["flake-edit.toml", ".flake-edit.toml"];

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub follow: FollowConfig,
}

/// Configuration for the `follow` command.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FollowConfig {
    /// Inputs to ignore during follow.
    #[serde(default)]
    pub ignore: Vec<String>,

    /// Minimum number of transitive follows needed to add a top-level follows input.
    /// Set to 0 to disable transitive follows deduplication.
    #[serde(default = "default_transitive_min")]
    pub transitive_min: usize,

    /// Alias mappings: canonical_name -> [alternative_names]
    /// e.g., nixpkgs = ["nixpkgs-lib"] means nixpkgs-lib can follow nixpkgs
    #[serde(default)]
    pub aliases: HashMap<String, Vec<String>>,
}

impl Default for FollowConfig {
    fn default() -> Self {
        Self {
            ignore: Vec::new(),
            transitive_min: default_transitive_min(),
            aliases: HashMap::new(),
        }
    }
}

impl FollowConfig {
    /// Check if an input should be ignored.
    ///
    /// Supports two formats:
    /// - Full path: `"crane.nixpkgs"` - ignores only that specific nested input
    /// - Simple name: `"nixpkgs"` - ignores all nested inputs with that name
    pub fn is_ignored(&self, path: &str, name: &str) -> bool {
        self.ignore.iter().any(|ignored| {
            // Check for full path match first (more specific)
            if ignored.contains('.') {
                ignored == path
            } else {
                // Simple name match
                ignored == name
            }
        })
    }

    /// Find the canonical name for a given input name.
    /// Returns the canonical name if found in aliases, otherwise returns None.
    pub fn resolve_alias(&self, name: &str) -> Option<&str> {
        for (canonical, alternatives) in &self.aliases {
            if alternatives.iter().any(|alt| alt == name) {
                return Some(canonical);
            }
        }
        None
    }

    /// Check if `nested_name` can follow `top_level_name`.
    /// Returns true if they match directly or via alias.
    pub fn can_follow(&self, nested_name: &str, top_level_name: &str) -> bool {
        // Direct match
        if nested_name == top_level_name {
            return true;
        }
        // Check if nested_name is an alias for top_level_name
        self.resolve_alias(nested_name) == Some(top_level_name)
    }

    pub fn transitive_min(&self) -> usize {
        self.transitive_min
    }
}

impl Config {
    /// Load configuration in the following order:
    /// 1. Project-level config (flake-edit.toml or .flake-edit.toml in current/parent dirs)
    /// 2. User-level config (~/.config/flake-edit/config.toml)
    /// 3. Default embedded config
    ///
    /// Returns an error if a config file exists but is malformed.
    pub fn load() -> Result<Self, ConfigError> {
        if let Some(path) = Self::project_config_path() {
            return Self::try_load_from_file(&path);
        }
        if let Some(path) = Self::user_config_path() {
            return Self::try_load_from_file(&path);
        }
        Ok(Self::default())
    }

    /// Load configuration from an explicitly specified path.
    ///
    /// Returns an error if the file doesn't exist or is malformed.
    /// If no path is specified, falls back to the default load order.
    pub fn load_from(path: Option<&Path>) -> Result<Self, ConfigError> {
        match path {
            Some(p) => Self::try_load_from_file(p),
            None => Self::load(),
        }
    }

    /// Try to load config from a file, returning detailed errors on failure.
    fn try_load_from_file(path: &Path) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path).map_err(|e| ConfigError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
        toml::from_str(&content).map_err(|e| ConfigError::Parse {
            path: path.to_path_buf(),
            source: e,
        })
    }

    pub fn project_config_path() -> Option<PathBuf> {
        let cwd = std::env::current_dir().ok()?;
        Self::find_config_in_ancestors(&cwd)
    }

    fn xdg_config_dir() -> Option<PathBuf> {
        let dirs = directories::ProjectDirs::from("", "", "flake-edit")?;
        Some(dirs.config_dir().to_path_buf())
    }

    pub fn user_config_path() -> Option<PathBuf> {
        let config_path = Self::xdg_config_dir()?.join("config.toml");
        config_path.exists().then_some(config_path)
    }

    pub fn user_config_dir() -> Option<PathBuf> {
        Self::xdg_config_dir()
    }

    fn find_config_in_ancestors(start: &Path) -> Option<PathBuf> {
        let mut current = start.to_path_buf();
        loop {
            for filename in CONFIG_FILENAMES {
                let config_path = current.join(filename);
                if config_path.exists() {
                    return Some(config_path);
                }
            }
            if !current.pop() {
                break;
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_parses() {
        let config: Config =
            toml::from_str(DEFAULT_CONFIG_TOML).expect("default config should parse");
        assert!(config.follow.ignore.is_empty());
        assert_eq!(config.follow.transitive_min, 2);
        assert!(config.follow.aliases.is_empty());
    }

    #[test]
    fn test_is_ignored_by_name() {
        let config = FollowConfig {
            ignore: vec!["flake-utils".to_string(), "systems".to_string()],
            ..Default::default()
        };

        // Simple name matching ignores all inputs with that name
        assert!(config.is_ignored("crane.flake-utils", "flake-utils"));
        assert!(config.is_ignored("poetry2nix.systems", "systems"));
        assert!(!config.is_ignored("crane.nixpkgs", "nixpkgs"));
    }

    #[test]
    fn test_is_ignored_by_path() {
        let config = FollowConfig {
            ignore: vec!["crane.nixpkgs".to_string()],
            ..Default::default()
        };

        // Full path matching only ignores that specific input
        assert!(config.is_ignored("crane.nixpkgs", "nixpkgs"));
        assert!(!config.is_ignored("poetry2nix.nixpkgs", "nixpkgs"));
    }

    #[test]
    fn test_is_ignored_mixed() {
        let config = FollowConfig {
            ignore: vec!["systems".to_string(), "crane.flake-utils".to_string()],
            ..Default::default()
        };

        // "systems" ignored everywhere
        assert!(config.is_ignored("crane.systems", "systems"));
        assert!(config.is_ignored("poetry2nix.systems", "systems"));

        // "flake-utils" only ignored for crane
        assert!(config.is_ignored("crane.flake-utils", "flake-utils"));
        assert!(!config.is_ignored("poetry2nix.flake-utils", "flake-utils"));
    }

    #[test]
    fn test_resolve_alias() {
        let config = FollowConfig {
            aliases: HashMap::from([(
                "nixpkgs".to_string(),
                vec!["nixpkgs-lib".to_string(), "nixpkgs-stable".to_string()],
            )]),
            ..Default::default()
        };

        assert_eq!(config.resolve_alias("nixpkgs-lib"), Some("nixpkgs"));
        assert_eq!(config.resolve_alias("nixpkgs-stable"), Some("nixpkgs"));
        assert_eq!(config.resolve_alias("nixpkgs"), None);
        assert_eq!(config.resolve_alias("unknown"), None);
    }

    #[test]
    fn test_can_follow_direct_match() {
        let config = FollowConfig::default();
        assert!(config.can_follow("nixpkgs", "nixpkgs"));
        assert!(!config.can_follow("nixpkgs", "flake-utils"));
    }

    #[test]
    fn test_can_follow_with_alias() {
        let config = FollowConfig {
            aliases: HashMap::from([("nixpkgs".to_string(), vec!["nixpkgs-lib".to_string()])]),
            ..Default::default()
        };

        // nixpkgs-lib can follow nixpkgs
        assert!(config.can_follow("nixpkgs-lib", "nixpkgs"));
        // direct match still works
        assert!(config.can_follow("nixpkgs", "nixpkgs"));
        // but not the reverse
        assert!(!config.can_follow("nixpkgs", "nixpkgs-lib"));
    }
}

fn default_transitive_min() -> usize {
    2
}
