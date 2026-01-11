use flake_edit::api::test_helpers::*;
use flake_edit::api::{ForgeType, IntermediaryTags, Tags};
use nix_uri::FlakeRef;

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

// URL parsing tests using nix-uri
#[test]
fn test_extract_domain_owner_repo_basic() {
    let url = "git+https://codeberg.org/forgejo/forgejo";
    let parsed: FlakeRef = url.parse().unwrap();
    assert_eq!(parsed.r#type.get_domain(), Some("codeberg.org".to_string()));
    assert_eq!(parsed.r#type.get_owner(), Some("forgejo".to_string()));
    assert_eq!(parsed.r#type.get_repo(), Some("forgejo".to_string()));
}

#[test]
fn test_extract_domain_owner_repo_with_git_suffix() {
    let url = "git+https://gitea.example.com/owner/repo.git";
    let parsed: FlakeRef = url.parse().unwrap();
    assert_eq!(
        parsed.r#type.get_domain(),
        Some("gitea.example.com".to_string())
    );
    assert_eq!(parsed.r#type.get_owner(), Some("owner".to_string()));
    assert_eq!(parsed.r#type.get_repo(), Some("repo".to_string()));
}

#[test]
fn test_extract_domain_owner_repo_subdomain() {
    let url = "git+https://git.example.com/myorg/myrepo";
    let parsed: FlakeRef = url.parse().unwrap();
    assert_eq!(
        parsed.r#type.get_domain(),
        Some("git.example.com".to_string())
    );
    assert_eq!(parsed.r#type.get_owner(), Some("myorg".to_string()));
    assert_eq!(parsed.r#type.get_repo(), Some("myrepo".to_string()));
}

#[test]
fn test_extract_domain_owner_repo_invalid_too_short() {
    let url = "git+https://example.com/owner";
    let parsed: FlakeRef = url.parse().unwrap();
    // Should have domain and owner but no repo
    assert_eq!(parsed.r#type.get_domain(), Some("example.com".to_string()));
    assert_eq!(parsed.r#type.get_owner(), Some("owner".to_string()));
    assert_eq!(parsed.r#type.get_repo(), None);
}

#[test]
fn test_extract_domain_owner_repo_localhost() {
    let url = "git+https://localhost:3000/test/project";
    let parsed: FlakeRef = url.parse().unwrap();
    assert_eq!(
        parsed.r#type.get_domain(),
        Some("localhost:3000".to_string())
    );
    assert_eq!(parsed.r#type.get_owner(), Some("test".to_string()));
    assert_eq!(parsed.r#type.get_repo(), Some("project".to_string()));
}

#[test]
fn test_extract_domain_owner_repo_ip_address() {
    let url = "git+https://192.168.1.1/owner/repo";
    let parsed: FlakeRef = url.parse().unwrap();
    assert_eq!(parsed.r#type.get_domain(), Some("192.168.1.1".to_string()));
    assert_eq!(parsed.r#type.get_owner(), Some("owner".to_string()));
    assert_eq!(parsed.r#type.get_repo(), Some("repo".to_string()));
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

#[test]
fn test_ref_or_rev_github() {
    let uri = "github:nix-community/home-manager";
    let ref_or_rev = "release-24.05";

    let mut flake_ref: FlakeRef = uri.parse().unwrap();
    flake_ref
        .r#type
        .ref_or_rev(Some(ref_or_rev.to_string()))
        .unwrap();

    assert_eq!(
        flake_ref.to_string(),
        "github:nix-community/home-manager/release-24.05"
    );
}

#[test]
fn test_ref_or_rev_gitlab() {
    let uri = "gitlab:owner/repo";
    let ref_or_rev = "main";

    let mut flake_ref: FlakeRef = uri.parse().unwrap();
    flake_ref
        .r#type
        .ref_or_rev(Some(ref_or_rev.to_string()))
        .unwrap();

    assert_eq!(flake_ref.to_string(), "gitlab:owner/repo/main");
}

#[test]
fn test_ref_or_rev_sourcehut() {
    let uri = "sourcehut:~user/repo";
    let ref_or_rev = "v1.0.0";

    let mut flake_ref: FlakeRef = uri.parse().unwrap();
    flake_ref
        .r#type
        .ref_or_rev(Some(ref_or_rev.to_string()))
        .unwrap();

    assert_eq!(flake_ref.to_string(), "sourcehut:~user/repo/v1.0.0");
}

#[test]
fn test_ref_or_rev_unsupported_type() {
    let uri = "git+https://github.com/example/repo";

    let mut flake_ref: FlakeRef = uri.parse().unwrap();
    let result = flake_ref.r#type.ref_or_rev(Some("main".to_string()));

    assert!(result.is_err());
}
