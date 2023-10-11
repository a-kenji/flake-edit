use crate::change::Change;

pub struct FlakeEdit {
    changes: Vec<Change>,
}

impl FlakeEdit {
    pub fn new(changes: Vec<Change>) -> Self {
        Self { changes }
    }
}
