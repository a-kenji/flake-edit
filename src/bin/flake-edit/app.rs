use std::fs::File;
use std::io;
use std::path::PathBuf;

use crate::cli::CliArgs;
use crate::error::FeError;
use crate::root::Root;
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
}

#[derive(Debug, Default)]
pub struct FlakeBuf {
    text: Rope,
    path: String,
}

impl FlakeBuf {
    fn from_path(path: PathBuf) -> io::Result<Self> {
        let text = Rope::from_reader(&mut io::BufReader::new(File::open(&path)?))?;
        let path = path.display().to_string();
        Ok(Self { text, path })
    }

    pub fn text(&self) -> &Rope {
        &self.text
    }
    pub fn apply(&self, change: &str) -> io::Result<()> {
        // println!("{}", self.path);
        std::fs::write(&self.path, change)?;
        // println!("{}", change);
        Ok(())
    }
}
