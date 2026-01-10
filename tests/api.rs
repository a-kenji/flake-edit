use flake_edit::api::test_helpers::*;
use flake_edit::api::{ForgeType, IntermediaryTags, Tags};
use flake_edit::uri::extract_domain_owner_repo_test;

#[test]
fn test_parse_forge_version_forgejo() {
    let json = r#"{"version":"1.21.0+forgejo"}"#;
    let result = parse_forge_version_test(json);
    assert_eq!(result, Some(ForgeType::Gitea));
}

#[test]
fn test_parse_forge_version_gitea_suffix() {
    let json = r#"{"version":"1.21.0+gitea"}"#;
    let result = parse_forge_version_test(json);
    assert_eq!(result, Some(ForgeType::Gitea));
}

#[test]
fn test_parse_forge_version_plain_version() {
    let json = r#"{"version":"1.21.0"}"#;
    let result = parse_forge_version_test(json);
    assert_eq!(result, None);
}

#[test]
fn test_parse_forge_version_invalid_json() {
    let json = r#"{"invalid": "data"}"#;
    let result = parse_forge_version_test(json);
    assert_eq!(result, None);
}

#[test]
fn test_parse_forge_version_malformed_json() {
    let json = r#"not json at all"#;
    let result = parse_forge_version_test(json);
    assert_eq!(result, None);
}

#[test]
fn test_extract_domain_owner_repo_basic() {
    let location = "codeberg.org/forgejo/forgejo";
    let result = extract_domain_owner_repo_test(location);
    assert_eq!(
        result,
        Some((
            "codeberg.org".to_string(),
            "forgejo".to_string(),
            "forgejo".to_string()
        ))
    );
}

#[test]
fn test_extract_domain_owner_repo_with_git_suffix() {
    let location = "gitea.example.com/owner/repo.git";
    let result = extract_domain_owner_repo_test(location);
    assert_eq!(
        result,
        Some((
            "gitea.example.com".to_string(),
            "owner".to_string(),
            "repo".to_string()
        ))
    );
}

#[test]
fn test_extract_domain_owner_repo_subdomain() {
    let location = "git.example.com/myorg/myrepo";
    let result = extract_domain_owner_repo_test(location);
    assert_eq!(
        result,
        Some((
            "git.example.com".to_string(),
            "myorg".to_string(),
            "myrepo".to_string()
        ))
    );
}

#[test]
fn test_extract_domain_owner_repo_invalid_too_short() {
    let location = "example.com/owner";
    let result = extract_domain_owner_repo_test(location);
    assert_eq!(result, None);
}

#[test]
fn test_extract_domain_owner_repo_invalid_one_part() {
    let location = "invalid";
    let result = extract_domain_owner_repo_test(location);
    assert_eq!(result, None);
}

#[test]
fn test_extract_domain_owner_repo_with_extra_path() {
    // Should only extract first three parts
    let location = "example.com/owner/repo/extra/path";
    let result = extract_domain_owner_repo_test(location);
    // The current implementation takes exactly 3 parts
    assert_eq!(
        result,
        Some((
            "example.com".to_string(),
            "owner".to_string(),
            "repo".to_string()
        ))
    );
}

#[test]
fn test_extract_domain_owner_repo_localhost() {
    let location = "localhost:3000/test/project";
    let result = extract_domain_owner_repo_test(location);
    assert_eq!(
        result,
        Some((
            "localhost:3000".to_string(),
            "test".to_string(),
            "project".to_string()
        ))
    );
}

#[test]
fn test_extract_domain_owner_repo_ip_address() {
    let location = "192.168.1.1/owner/repo";
    let result = extract_domain_owner_repo_test(location);
    assert_eq!(
        result,
        Some((
            "192.168.1.1".to_string(),
            "owner".to_string(),
            "repo".to_string()
        ))
    );
}

#[test]
fn test_extract_domain_owner_repo_empty_string() {
    let location = "";
    let result = extract_domain_owner_repo_test(location);
    assert_eq!(result, None);
}

// Tag parsing tests
#[test]
fn test_tags_parsing_with_refs_tags_prefix() {
    let json = r#"[
        {"name": "refs/tags/v1.0.0"},
        {"name": "refs/tags/v2.0.0"},
        {"name": "refs/tags/v1.5.0"}
    ]"#;

    let intermediary: IntermediaryTags = serde_json::from_str(json).unwrap();
    let mut tags: Tags = intermediary.into();

    assert_eq!(tags.get_latest_tag(), Some("refs/tags/v2.0.0".to_string()));
}

#[test]
fn test_tags_parsing_with_v_prefix() {
    let json = r#"[
        {"name": "v1.0.0"},
        {"name": "v2.0.0"},
        {"name": "v1.5.0"}
    ]"#;

    let intermediary: IntermediaryTags = serde_json::from_str(json).unwrap();
    let mut tags: Tags = intermediary.into();

    assert_eq!(tags.get_latest_tag(), Some("v2.0.0".to_string()));
}

#[test]
fn test_tags_parsing_with_short_versions() {
    let json = r#"[
        {"name": "v1"},
        {"name": "v1.1"}
    ]"#;

    let intermediary: IntermediaryTags = serde_json::from_str(json).unwrap();
    let mut tags: Tags = intermediary.into();

    assert_eq!(tags.get_latest_tag(), Some("v1.1".to_string()));
}

#[test]
fn test_tags_parsing_without_prefix() {
    let json = r#"[
        {"name": "1.0.0"},
        {"name": "2.0.0"},
        {"name": "1.5.0"}
    ]"#;

    let intermediary: IntermediaryTags = serde_json::from_str(json).unwrap();
    let mut tags: Tags = intermediary.into();

    assert_eq!(tags.get_latest_tag(), Some("2.0.0".to_string()));
}

#[test]
fn test_tags_parsing_with_dash_prefix() {
    let json = r#"[
        {"name": "release-1.0.0"},
        {"name": "release-2.0.0"},
        {"name": "release-1.5.0"}
    ]"#;

    let intermediary: IntermediaryTags = serde_json::from_str(json).unwrap();
    let mut tags: Tags = intermediary.into();

    assert_eq!(tags.get_latest_tag(), Some("release-2.0.0".to_string()));
}

#[test]
fn test_tags_parsing_mixed_valid_invalid() {
    let json = r#"[
        {"name": "v1.0.0"},
        {"name": "v2.0.0"},
        {"name": "invalid-tag"},
        {"name": "v1.5.0"}
    ]"#;

    let intermediary: IntermediaryTags = serde_json::from_str(json).unwrap();
    let mut tags: Tags = intermediary.into();

    // Should only parse valid semver tags
    assert_eq!(tags.get_latest_tag(), Some("v2.0.0".to_string()));
}

#[test]
fn test_tags_parsing_empty() {
    let json = r#"[]"#;

    let intermediary: IntermediaryTags = serde_json::from_str(json).unwrap();
    let mut tags: Tags = intermediary.into();

    assert_eq!(tags.get_latest_tag(), None);
}

#[test]
fn test_tags_parsing_prerelease_versions() {
    let json = r#"[
        {"name": "v1.0.0"},
        {"name": "v2.0.0-beta.1"},
        {"name": "v1.5.0"}
    ]"#;

    let intermediary: IntermediaryTags = serde_json::from_str(json).unwrap();
    let mut tags: Tags = intermediary.into();

    // Should handle prerelease versions
    let latest = tags.get_latest_tag();
    assert!(latest.is_some());
}

#[test]
fn test_tags_parsing_combined_prefixes() {
    // Test refs/tags/ + v prefix combination
    let json = r#"[
        {"name": "refs/tags/v1.0.0"},
        {"name": "refs/tags/v2.0.0"}
    ]"#;

    let intermediary: IntermediaryTags = serde_json::from_str(json).unwrap();
    let mut tags: Tags = intermediary.into();

    assert_eq!(tags.get_latest_tag(), Some("refs/tags/v2.0.0".to_string()));
}

#[test]
fn test_tags_sorting_order() {
    let json = r#"[
        {"name": "v10.0.0"},
        {"name": "v2.0.0"},
        {"name": "v1.0.0"}
    ]"#;

    let intermediary: IntermediaryTags = serde_json::from_str(json).unwrap();
    let mut tags: Tags = intermediary.into();

    // Should correctly sort by semver, not lexicographically
    assert_eq!(tags.get_latest_tag(), Some("v10.0.0".to_string()));
}
