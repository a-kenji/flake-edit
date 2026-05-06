use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Default configuration TOML embedded in the binary.
pub const DEFAULT_CONFIG_TOML: &str = include_str!("assets/config.toml");

/// Configuration loading failures.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// Failed to read the configuration file from disk.
    #[error("Failed to read config file '{path}': {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// Failed to parse a configuration file as TOML.
    #[error("Failed to parse config file '{path}':\n{source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
}

/// Filenames searched for project-level configuration, in priority order.
const CONFIG_FILENAMES: &[&str] = &["flake-edit.toml", ".flake-edit.toml"];

/// Top-level `flake-edit.toml` configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub follow: FollowConfig,
}

/// `[follow]` section of [`Config`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FollowConfig {
    /// Inputs to skip during follow analysis.
    ///
    /// See [`Self::is_ignored`] for the matching rules.
    #[serde(default)]
    pub ignore: Vec<String>,

    /// Minimum number of transitive references required before a shared
    /// nested input is promoted to top-level. `0` disables transitive
    /// deduplication.
    #[serde(default = "default_transitive_min")]
    pub transitive_min: usize,

    /// Alias mappings: canonical name to alternative names. For example,
    /// `nixpkgs = ["nixpkgs-lib"]` lets `nixpkgs-lib` follow `nixpkgs`.
    #[serde(default)]
    pub aliases: HashMap<String, Vec<String>>,

    /// Maximum depth of follows declarations to write.
    ///
    /// `1` (default) writes only `parent.nested.follows = "target"`. `2` or
    /// higher also writes deeper paths such as
    /// `parent.middle.grandchild.follows = "target"`.
    #[serde(default = "default_max_depth")]
    pub max_depth: usize,
}

impl Default for FollowConfig {
    fn default() -> Self {
        Self {
            ignore: Vec::new(),
            transitive_min: default_transitive_min(),
            aliases: HashMap::new(),
            max_depth: default_max_depth(),
        }
    }
}

impl FollowConfig {
    /// True if the input at `path` (e.g. `crane.nixpkgs`) with simple `name`
    /// (e.g. `nixpkgs`) is in [`Self::ignore`].
    ///
    /// Entries containing a `.` match the full dotted path. Bare entries
    /// match by name across all parents.
    pub fn is_ignored(&self, path: &str, name: &str) -> bool {
        self.ignore.iter().any(|ignored| {
            if ignored.contains('.') {
                ignored == path
            } else {
                ignored == name
            }
        })
    }

    /// Canonical name `name` is an alias of, or `None` if no alias applies.
    pub fn resolve_alias(&self, name: &str) -> Option<&str> {
        for (canonical, alternatives) in &self.aliases {
            if alternatives.iter().any(|alt| alt == name) {
                return Some(canonical);
            }
        }
        None
    }

    /// True if `nested_name` may follow `top_level_name` (direct match or via
    /// [`Self::aliases`]).
    pub fn can_follow(&self, nested_name: &str, top_level_name: &str) -> bool {
        if nested_name == top_level_name {
            return true;
        }
        self.resolve_alias(nested_name) == Some(top_level_name)
    }

    pub fn transitive_min(&self) -> usize {
        self.transitive_min
    }
}

impl Config {
    /// Load the first available configuration:
    /// 1. Project-level ([`CONFIG_FILENAMES`], walking upward from the
    ///    current directory).
    /// 2. User-level (`~/.config/flake-edit/config.toml`).
    /// 3. The default embedded config.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] if a discovered file cannot be read or
    /// parsed.
    pub fn load() -> Result<Self, ConfigError> {
        if let Some(path) = Self::project_config_path() {
            return Self::try_load_from_file(&path);
        }
        if let Some(path) = Self::user_config_path() {
            return Self::try_load_from_file(&path);
        }
        Ok(Self::default())
    }

    /// Load configuration from `path`, or fall back to [`Self::load`] when
    /// `path` is `None`.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] if `path` does not exist or cannot be parsed.
    pub fn load_from(path: Option<&Path>) -> Result<Self, ConfigError> {
        match path {
            Some(p) => Self::try_load_from_file(p),
            None => Self::load(),
        }
    }

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

    /// Path to the nearest project-level config file, walking upward from
    /// the current directory.
    pub fn project_config_path() -> Option<PathBuf> {
        let cwd = std::env::current_dir().ok()?;
        Self::find_config_in_ancestors(&cwd)
    }

    fn xdg_config_dir() -> Option<PathBuf> {
        let dirs = directories::ProjectDirs::from("", "", "flake-edit")?;
        Some(dirs.config_dir().to_path_buf())
    }

    /// Path to `~/.config/flake-edit/config.toml`, or `None` if it does not
    /// exist.
    pub fn user_config_path() -> Option<PathBuf> {
        let config_path = Self::xdg_config_dir()?.join("config.toml");
        config_path.exists().then_some(config_path)
    }

    /// XDG config directory for flake-edit, regardless of whether it exists.
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

fn default_transitive_min() -> usize {
    0
}

fn default_max_depth() -> usize {
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_parses() {
        let config: Config =
            toml::from_str(DEFAULT_CONFIG_TOML).expect("default config should parse");
        assert!(config.follow.ignore.is_empty());
        assert_eq!(config.follow.transitive_min, 0);
        assert!(config.follow.aliases.is_empty());
        assert_eq!(config.follow.max_depth, 1);
    }

    #[test]
    fn max_depth_defaults_to_one() {
        let cfg = FollowConfig::default();
        assert_eq!(cfg.max_depth, 1);
    }

    #[test]
    fn max_depth_parses_from_toml() {
        let cfg: Config = toml::from_str("[follow]\nmax_depth = 2\n").unwrap();
        assert_eq!(cfg.follow.max_depth, 2);
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
