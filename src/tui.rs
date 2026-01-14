use std::io::{self, IsTerminal};

pub mod app;
mod backend;
pub mod components;
mod helpers;
mod run;
mod style;
mod view;
pub mod workflow;

pub use app::App;
pub use run::run;
pub use workflow::{AppResult, ConfirmResultAction, MultiSelectResultData, SingleSelectResult};

pub fn is_interactive(non_interactive_flag: bool) -> bool {
    if non_interactive_flag {
        return false;
    }
    if std::env::var("CI").is_ok() {
        return false;
    }
    io::stdout().is_terminal()
}
