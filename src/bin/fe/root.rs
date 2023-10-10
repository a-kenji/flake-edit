use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::{env, fs};

use tracing::debug;

use crate::error::FeError;

#[derive(Debug, Clone, PartialEq)]
pub struct Root(PathBuf);

impl Root {
    pub fn path(&self) -> &PathBuf {
        &self.0
    }
    /// Emulate default nix path searching behaviour,
    /// first check if the path is in the same directory,
    /// then seek upwards until either the file is found,
    /// or the root is reached.
    pub fn from_path<P>(path: P) -> Result<Root, FeError>
    where
        P: AsRef<Path>,
    {
        if fs::metadata(path.as_ref()).is_ok() {
            return Ok(Root(PathBuf::from(path.as_ref())));
        } else {
            Self::find_root(path)
        }
    }
    fn find_root<P>(path: P) -> Result<Root, FeError>
    where
        P: AsRef<Path>,
    {
        let mut cwd = Self::from_cwd()?;
        cwd.0.push(&path);
        let git_root = Self::from_git()?;

        tracing::debug!("{cwd:?}");
        while let Err(_meta) = fs::metadata(cwd.path().join(&path)) {
            tracing::debug!("{cwd:?}");
            if cwd == git_root {
                return Err(FeError::Error("Already at git the git root.".into()));
            }
            if !cwd.0.pop() {
                return Err(FeError::Error("The Path has no parent anymore.".into()));
            }
        }
        cwd.0.push(&path);
        Ok(cwd)
    }
    fn from_cwd() -> Result<Root, FeError> {
        let cwd = env::current_dir()?;
        Ok(Root(cwd))
    }
    fn from_git() -> Result<Root, FeError> {
        let output = Command::new("git")
            .arg("rev-parse")
            .arg("--show-toplevel")
            // TODO: maybe try as alternative,
            // if first fails.
            // .arg("--show-cdup")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .expect("Failed to execute command");

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            debug!("Command output:\n{}", stdout);
            Ok(Root(PathBuf::from(stdout.as_ref())))
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::error!("Error executing command:\n{}", stderr);
            Err(FeError::Error(stderr.to_string()))
        }
    }
}
