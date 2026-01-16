//! URI completion helpers for TUI input fields.
//!
//! This module provides completion items for flake URI input, supporting:
//! - ID-specific URIs from cache (for Change workflow toggling between remote/local)
//! - Owner/org prefixes extracted from cached URIs (e.g., `github:mic92/`)
//! - Default URI type prefixes (github:, path:, etc.)
//! - General cached URIs sorted by usage frequency

use std::collections::HashSet;

use crate::cache::{Cache, CacheConfig, DEFAULT_URI_TYPES};

/// Extract the owner/org prefix from a flake URI.
///
/// This extracts prefixes like `github:mic92/` from `github:mic92/vmsh`,
/// enabling completion of different repos from the same owner.
///
/// # Supported patterns
///
/// - `github:owner/repo` → `github:owner/`
/// - `gitlab:owner/repo` → `gitlab:owner/`
/// - `sourcehut:~owner/repo` → `sourcehut:~owner/`
/// - `git+https://github.com/owner/repo` → `git+https://github.com/owner/`
/// - `git+https://gitlab.com/owner/repo` → `git+https://gitlab.com/owner/`
///
/// Returns `None` for URIs without an extractable owner (e.g., `path:`, `flake:`).
fn extract_owner_prefix(uri: &str) -> Option<String> {
    // Handle github:/gitlab:/sourcehut: shorthand
    for prefix in ["github:", "gitlab:"] {
        if let Some(rest) = uri.strip_prefix(prefix) {
            // Find the first slash (owner/repo separator)
            if let Some(slash_pos) = rest.find('/') {
                // Only extract if there's actual content after the slash (it's a full URI)
                if slash_pos > 0 && rest.len() > slash_pos + 1 {
                    return Some(format!("{}{}/", prefix, &rest[..slash_pos]));
                }
            }
        }
    }

    // Handle sourcehut with ~ prefix for users
    if let Some(rest) = uri.strip_prefix("sourcehut:")
        && let Some(slash_pos) = rest.find('/')
        && slash_pos > 0
        && rest.len() > slash_pos + 1
    {
        return Some(format!("sourcehut:{}/", &rest[..slash_pos]));
    }

    // Handle git+https:// URLs (GitHub, GitLab, etc.)
    for host in ["github.com", "gitlab.com", "codeberg.org"] {
        let pattern = format!("git+https://{}/", host);
        if let Some(rest) = uri.strip_prefix(&pattern)
            && let Some(slash_pos) = rest.find('/')
            && slash_pos > 0
            && rest.len() > slash_pos + 1
        {
            return Some(format!("{}{}/", pattern, &rest[..slash_pos]));
        }
    }

    None
}

/// Build completion items for URI input.
///
/// When `id` is provided (e.g., in the Change workflow), URIs previously used
/// for that specific input ID are prepended to the list. This supports the
/// common workflow of toggling between a remote URI and a local path for testing.
///
/// # Completion Order
///
/// 1. **ID-specific URIs** (if `id` is `Some`) - URIs previously used for this
///    exact input ID, sorted by hit count. Enables quick toggling between
///    `github:owner/repo` and `path:/local/checkout`.
///
/// 2. **Default URI type prefixes** - The 14 standard flake URI schemes like
///    `github:`, `gitlab:`, `path:`, `git+https://`, etc.
///
/// 3. **Owner/org prefixes** - Extracted from cached URIs (e.g., `github:mic92/`
///    from `github:mic92/vmsh`). Enables quick access to other repos from the
///    same owner.
///
/// 4. **General cached URIs** - All other previously used URIs from the global
///    cache, sorted by hit count, excluding duplicates.
pub fn uri_completion_items(id: Option<&str>, cache_config: &CacheConfig) -> Vec<String> {
    let mut items: Vec<String> = Vec::new();
    // Track seen items for O(1) deduplication instead of O(n) contains() checks
    let mut seen: HashSet<String> = HashSet::new();

    let cache = match cache_config {
        CacheConfig::Default => Some(Cache::load()),
        CacheConfig::Custom(path) => Some(Cache::from_path(path)),
        CacheConfig::None => None,
    };

    if let Some(cache) = cache {
        let cached_uris = cache.list_uris();

        // Prepend ID-specific URIs (for change workflow)
        if let Some(id) = id {
            for uri in cache.list_uris_for_id(id) {
                if seen.insert(uri.clone()) {
                    items.push(uri);
                }
            }
        }

        // Add default URI type prefixes
        for uri_type in DEFAULT_URI_TYPES {
            let s = uri_type.to_string();
            if seen.insert(s.clone()) {
                items.push(s);
            }
        }

        // Extract and add unique owner prefixes from cached URIs
        let mut owner_prefixes: Vec<String> = cached_uris
            .iter()
            .filter_map(|uri| extract_owner_prefix(uri))
            .filter(|prefix| !seen.contains(prefix))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        // Sort prefixes for consistent ordering
        owner_prefixes.sort();
        for prefix in owner_prefixes {
            seen.insert(prefix.clone());
            items.push(prefix);
        }

        // Add general cached URIs (excluding duplicates)
        for uri in cached_uris {
            if seen.insert(uri.clone()) {
                items.push(uri);
            }
        }
    } else {
        // No cache - just return default URI types
        items.extend(DEFAULT_URI_TYPES.iter().map(|s| s.to_string()));
    }

    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_owner_prefix_github() {
        assert_eq!(
            extract_owner_prefix("github:mic92/vmsh"),
            Some("github:mic92/".to_string())
        );
        assert_eq!(
            extract_owner_prefix("github:NixOS/nixpkgs"),
            Some("github:NixOS/".to_string())
        );
        assert_eq!(
            extract_owner_prefix("github:nix-community/home-manager"),
            Some("github:nix-community/".to_string())
        );
    }

    #[test]
    fn test_extract_owner_prefix_gitlab() {
        assert_eq!(
            extract_owner_prefix("gitlab:someorg/project"),
            Some("gitlab:someorg/".to_string())
        );
    }

    #[test]
    fn test_extract_owner_prefix_sourcehut() {
        assert_eq!(
            extract_owner_prefix("sourcehut:~user/repo"),
            Some("sourcehut:~user/".to_string())
        );
    }

    #[test]
    fn test_extract_owner_prefix_git_https() {
        assert_eq!(
            extract_owner_prefix("git+https://github.com/owner/repo"),
            Some("git+https://github.com/owner/".to_string())
        );
        assert_eq!(
            extract_owner_prefix("git+https://gitlab.com/org/project"),
            Some("git+https://gitlab.com/org/".to_string())
        );
        assert_eq!(
            extract_owner_prefix("git+https://codeberg.org/user/repo"),
            Some("git+https://codeberg.org/user/".to_string())
        );
    }

    #[test]
    fn test_extract_owner_prefix_none() {
        // URIs without extractable owner
        assert_eq!(extract_owner_prefix("path:/some/local/path"), None);
        assert_eq!(extract_owner_prefix("flake:nixpkgs"), None);
        assert_eq!(extract_owner_prefix("github:"), None);
        assert_eq!(extract_owner_prefix("github:owner"), None); // No repo part
        assert_eq!(extract_owner_prefix("github:owner/"), None); // Empty repo
    }

    #[test]
    fn test_extract_owner_prefix_with_query_params() {
        // Should still extract owner even with query params
        assert_eq!(
            extract_owner_prefix("github:NixOS/nixpkgs?ref=nixos-unstable"),
            Some("github:NixOS/".to_string())
        );
    }
}
