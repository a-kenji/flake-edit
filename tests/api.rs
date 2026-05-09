use nix_uri::FlakeRef;

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
