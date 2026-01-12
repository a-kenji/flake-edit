use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::{env, fs};

use tracing::debug;

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
    pub fn from_path<P>(path: P) -> Result<Root, std::io::Error>
    where
        P: AsRef<Path>,
    {
        if fs::metadata(path.as_ref()).is_ok() {
            Ok(Root(PathBuf::from(path.as_ref())))
        } else {
            Self::find_root(path)
        }
    }

    fn find_root<P>(path: P) -> Result<Root, std::io::Error>
    where
        P: AsRef<Path>,
    {
        let mut cwd = Self::from_cwd()?;
        cwd.0.push(&path);
        let git_root = Self::from_git().ok();

        tracing::debug!("{cwd:?}");
        while fs::metadata(cwd.path().join(&path)).is_err() {
            tracing::debug!("{cwd:?}");
            if git_root.as_ref() == Some(&cwd) {
                return Err(std::io::Error::other("Already at the git root"));
            }
            if !cwd.0.pop() {
                return Err(std::io::Error::other("The path has no parent anymore"));
            }
        }
        cwd.0.push(&path);
        Ok(cwd)
    }

    fn from_cwd() -> Result<Root, std::io::Error> {
        let cwd = env::current_dir()?;
        Ok(Root(cwd))
    }

    fn from_git() -> Result<Root, std::io::Error> {
        let output = Command::new("git")
            .arg("rev-parse")
            .arg("--show-toplevel")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            debug!("Command output:\n{}", stdout);
            Ok(Root(PathBuf::from(stdout.trim())))
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::error!("Error executing command:\n{}", stderr);
            Err(std::io::Error::other(stderr.to_string()))
        }
    }
}
