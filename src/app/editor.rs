use std::fs::File;
use std::io;
use std::path::PathBuf;
use std::process::Command;

use ropey::Rope;

use crate::diff::Diff;
use crate::edit::FlakeEdit;
use crate::error::FlakeEditError;
use crate::validate;

use super::state::AppState;

/// Buffer for a flake file with its content and path.
#[derive(Debug, Default)]
pub struct FlakeBuf {
    text: Rope,
    path: PathBuf,
}

impl FlakeBuf {
    pub fn from_path(path: PathBuf) -> io::Result<Self> {
        let text = Rope::from_reader(&mut io::BufReader::new(File::open(&path)?))?;
        Ok(Self { text, path })
    }

    pub fn text(&self) -> &Rope {
        &self.text
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub fn write(&self, content: &str) -> io::Result<()> {
        std::fs::write(&self.path, content)
    }
}

/// Editor that drives changes to flake.nix files.
///
/// Handles file I/O, applying changes, and running nix flake lock.
#[derive(Debug)]
pub struct Editor {
    flake: FlakeBuf,
}

impl Editor {
    pub fn new(flake: FlakeBuf) -> Self {
        Self { flake }
    }

    pub fn from_path(path: PathBuf) -> io::Result<Self> {
        let flake = FlakeBuf::from_path(path)?;
        Ok(Self { flake })
    }

    pub fn text(&self) -> String {
        self.flake.text().to_string()
    }

    pub fn path(&self) -> &PathBuf {
        self.flake.path()
    }

    pub fn create_flake_edit(&self) -> Result<FlakeEdit, FlakeEditError> {
        FlakeEdit::from_text(&self.text())
    }

    fn run_nix_flake_lock(&self) -> io::Result<()> {
        let flake_dir = match self.flake.path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
            _ => PathBuf::from("."),
        };

        let output = Command::new("nix")
            .args(["flake", "lock"])
            .current_dir(&flake_dir)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            eprintln!("Warning: nix flake lock failed: {}", stderr);
            return Err(io::Error::other(format!(
                "nix flake lock failed: {}",
                stderr
            )));
        }

        println!("Updated flake.lock");
        Ok(())
    }

    /// Apply changes to the flake file, or show diff if in diff mode.
    ///
    /// Validates the new content for duplicate attributes before writing.
    pub fn apply_or_diff(&self, new_content: &str, state: &AppState) -> Result<(), FlakeEditError> {
        let validation = validate::validate(new_content);
        if validation.has_errors() {
            return Err(FlakeEditError::Validation(validation.errors));
        }

        if state.diff {
            let old = self.text();
            let diff = Diff::new(&old, new_content);
            diff.compare();
        } else {
            self.flake.write(new_content)?;

            if !state.no_lock
                && let Err(e) = self.run_nix_flake_lock()
            {
                eprintln!("Warning: Failed to update lockfile: {}", e);
            }
        }
        Ok(())
    }
}
