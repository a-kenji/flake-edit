pub fn is_git_url(uri: &str) -> bool {
    uri.starts_with("git+https://") || uri.starts_with("git+http://")
}
