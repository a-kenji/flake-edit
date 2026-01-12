//! Wrapper for diffing the changes

use std::io::IsTerminal;
pub struct Diff<'a> {
    old: &'a str,
    new: &'a str,
}

fn use_color() -> bool {
    // Respect NO_COLOR (https://no-color.org/)
    if std::env::var("NO_COLOR").is_ok() {
        return false;
    }
    std::io::stdout().is_terminal()
}

impl<'a> Diff<'a> {
    pub fn new(old: &'a str, new: &'a str) -> Self {
        Self { old, new }
    }
    pub fn compare(&self) {
        print!("{}", self.to_string_colored(use_color()));
    }
    /// Return the diff as a string, optionally with ANSI colors
    pub fn to_string_colored(&self, color: bool) -> String {
        let patch = diffy::create_patch(self.old, self.new);
        let f = if color {
            diffy::PatchFormatter::new().with_color()
        } else {
            diffy::PatchFormatter::new()
        };
        f.fmt_patch(&patch).to_string()
    }
    /// Return the diff as a plain string without colors
    pub fn to_string_plain(&self) -> String {
        self.to_string_colored(false)
    }
}
