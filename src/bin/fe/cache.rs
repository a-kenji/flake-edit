use std::path::PathBuf;

use directories::ProjectDirs;

pub fn cache_dir() -> Option<PathBuf> {
    if let Some(project_dir) = ProjectDirs::from("com", "a-kenji", "fe") {
        return Some(project_dir.data_dir().to_path_buf());
    }
    None
}

pub const fn default_types() -> [&'static str; 2] {
    ["github", "gitlab"]
}
