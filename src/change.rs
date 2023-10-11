#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub enum Change {
    #[default]
    None,
    Add {
        id: Option<String>,
        uri: Option<String>,
    },
    Remove {
        id: String,
    },
    Pin {
        id: String,
    },
    Change {
        id: Option<String>,
        ref_or_rev: Option<String>,
    },
}

impl Change {
    pub fn id(&self) -> Option<String> {
        match self {
            Change::None => None,
            Change::Add { id, .. } => id.clone(),
            Change::Remove { id } => Some(id.clone()),
            Change::Change { id, .. } => id.clone(),
            Change::Pin { id } => Some(id.clone()),
        }
    }
    pub fn is_remove(&self) -> bool {
        matches!(self, Change::Remove { .. })
    }
    pub fn is_some(&self) -> bool {
        !matches!(self, Change::None)
    }
}
