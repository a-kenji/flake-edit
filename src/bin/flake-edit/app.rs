use std::fs::File;
use std::io;
use std::path::PathBuf;
use std::process::Command;

use crate::cli::CliArgs;
use crate::error::FeError;
use crate::root::Root;
use flake_edit::diff::Diff;
use rnix::tokenizer::Tokenizer;
use ropey::Rope;

#[derive(Debug, Default)]
pub struct FlakeEdit {
    pub root: FlakeBuf,
    _lock: Option<FlakeBuf>,
}

impl FlakeEdit {
    const FLAKE: &'static str = "flake.nix";
    pub fn init(args: &CliArgs) -> Result<Self, FeError> {
        let path = if let Some(flake) = args.flake() {
            PathBuf::from(flake)
        } else {
            let path = PathBuf::from(Self::FLAKE);
            let binding = Root::from_path(path)?;
            let root = binding.path();
            root.to_path_buf()
        };
        let root = FlakeBuf::from_path(path)?;
        Ok(Self { root, _lock: None })
    }

    pub fn root(&self) -> &FlakeBuf {
        &self.root
    }

    pub fn text(&self) -> String {
        self.root().text().to_string()
    }

    pub fn create_editor(&self) -> Result<crate::edit::FlakeEdit, FeError> {
        let text = self.root().text().to_string();
        let (_node, errors) = rnix::parser::parse(Tokenizer::new(&text));
        if !errors.is_empty() {
            tracing::error!("There are errors in the root document.");
        }
        Ok(crate::edit::FlakeEdit::from_text(&text)?)
    }

    fn run_nix_flake_lock(&self) -> io::Result<()> {
        let flake_path = PathBuf::from(&self.root.path);
        let flake_dir = match flake_path.parent() {
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

    /// Apply pending changes to the FlakeBuf,
    /// if specified only diff the changes and don't apply.
    pub fn apply_change_or_diff(
        &self,
        change: &str,
        diff: bool,
        no_lock: bool,
    ) -> Result<(), FeError> {
        if diff {
            let old = self.text();
            let diff = Diff::new(&old, change);
            diff.compare();
        } else {
            self.root.apply(change)?;

            if !no_lock && let Err(e) = self.run_nix_flake_lock() {
                // Make lock failures non-fatal warnings
                eprintln!("Warning: Failed to update lockfile: {}", e);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct FlakeBuf {
    text: Rope,
    path: String,
}

impl FlakeBuf {
    fn from_path(path: PathBuf) -> io::Result<Self> {
        let text = Rope::from_reader(&mut io::BufReader::new(File::open(&path)?))?;
        let path_str = path.display().to_string();
        Ok(Self {
            text,
            path: path_str,
        })
    }

    pub fn text(&self) -> &Rope {
        &self.text
    }

    pub fn apply(&self, change: &str) -> io::Result<()> {
        std::fs::write(&self.path, change)?;
        Ok(())
    }
}
