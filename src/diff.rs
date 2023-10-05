//! Wrapper for diffing the changes
pub struct Diff<'a> {
    old: &'a str,
    new: &'a str,
}

impl<'a> Diff<'a> {
    pub fn new(old: &'a str, new: &'a str) -> Self {
        Self { old, new }
    }
    pub fn compare(&self) {
        let patch = diffy::create_patch(self.old, self.new);
        let f = diffy::PatchFormatter::new().with_color();
        print!("{}", f.fmt_patch(&patch));
    }
}
