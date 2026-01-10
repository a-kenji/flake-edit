pub fn is_git_url(uri: &str) -> bool {
    uri.starts_with("git+https://") || uri.starts_with("git+http://")
}

/// Extract domain, owner, and repo from a git+https URL location
/// Handles patterns like:
/// - "codeberg.org/forgejo/forgejo"
/// - "gitea.example.com/owner/repo"
/// - "gitea.example.com/owner/repo.git"
pub fn extract_domain_owner_repo(location: &str) -> Option<(String, String, String)> {
    let parts: Vec<&str> = location.split('/').collect();
    if parts.len() < 3 {
        return None;
    }

    let domain = parts[0].to_string();
    let owner = parts[1].to_string();
    let mut repo = parts[2].to_string();

    // Strip .git suffix if present
    if repo.ends_with(".git") {
        repo = repo.strip_suffix(".git").unwrap().to_string();
    }

    Some((domain, owner, repo))
}

#[doc(hidden)]
pub fn extract_domain_owner_repo_test(location: &str) -> Option<(String, String, String)> {
    extract_domain_owner_repo(location)
}
