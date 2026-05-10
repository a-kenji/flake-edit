use nix_uri::FlakeRef;

// URL parsing tests using nix-uri
#[test]
fn test_extract_domain_owner_repo_basic() {
    let url = "git+https://codeberg.org/forgejo/forgejo";
    let parsed: FlakeRef = url.parse().unwrap();
    assert_eq!(parsed.domain(), Some("codeberg.org"));
    assert_eq!(parsed.owner(), Some("forgejo"));
    assert_eq!(parsed.repo(), Some("forgejo"));
}

#[test]
fn test_extract_domain_owner_repo_with_git_suffix() {
    let url = "git+https://gitea.example.com/owner/repo.git";
    let parsed: FlakeRef = url.parse().unwrap();
    assert_eq!(parsed.domain(), Some("gitea.example.com"));
    assert_eq!(parsed.owner(), Some("owner"));
    assert_eq!(parsed.repo(), Some("repo"));
}

#[test]
fn test_extract_domain_owner_repo_subdomain() {
    let url = "git+https://git.example.com/myorg/myrepo";
    let parsed: FlakeRef = url.parse().unwrap();
    assert_eq!(parsed.domain(), Some("git.example.com"));
    assert_eq!(parsed.owner(), Some("myorg"));
    assert_eq!(parsed.repo(), Some("myrepo"));
}

#[test]
fn test_extract_domain_owner_repo_invalid_too_short() {
    let url = "git+https://example.com/owner";
    let parsed: FlakeRef = url.parse().unwrap();
    // Should have domain and owner but no repo
    assert_eq!(parsed.domain(), Some("example.com"));
    assert_eq!(parsed.owner(), Some("owner"));
    assert_eq!(parsed.repo(), None);
}

#[test]
fn test_extract_domain_owner_repo_localhost() {
    // Non-default ports thread through `domain()` rather than landing on
    // a separate accessor, so `forge::api` has one authority string to
    // interpolate, not a host plus port pair.
    let url = "git+https://localhost:3000/test/project";
    let parsed: FlakeRef = url.parse().unwrap();
    assert_eq!(parsed.domain(), Some("localhost:3000"));
    assert_eq!(parsed.owner(), Some("test"));
    assert_eq!(parsed.repo(), Some("project"));
}

#[test]
fn test_extract_domain_default_port_drops_port() {
    // Default-port `host:443` is URL-normalised away so `domain()` is
    // byte-equivalent whether the user typed the port or not.
    let url = "git+https://gitea.example.com:443/owner/repo";
    let parsed: FlakeRef = url.parse().unwrap();
    assert_eq!(parsed.domain(), Some("gitea.example.com"));
    assert_eq!(parsed.owner(), Some("owner"));
    assert_eq!(parsed.repo(), Some("repo"));
}

#[test]
fn test_extract_domain_owner_repo_ip_address() {
    let url = "git+https://192.168.1.1/owner/repo";
    let parsed: FlakeRef = url.parse().unwrap();
    assert_eq!(parsed.domain(), Some("192.168.1.1"));
    assert_eq!(parsed.owner(), Some("owner"));
    assert_eq!(parsed.repo(), Some("repo"));
}

#[test]
fn test_set_ref_github() {
    let uri = "github:nix-community/home-manager";
    let updated = uri
        .parse::<FlakeRef>()
        .unwrap()
        .with_ref(Some("release-24.05".to_string()))
        .into_uri();

    assert_eq!(updated, "github:nix-community/home-manager/release-24.05");
}

#[test]
fn test_set_ref_gitlab() {
    let uri = "gitlab:owner/repo";
    let updated = uri
        .parse::<FlakeRef>()
        .unwrap()
        .with_ref(Some("main".to_string()))
        .into_uri();

    assert_eq!(updated, "gitlab:owner/repo/main");
}

#[test]
fn test_set_ref_sourcehut() {
    let uri = "sourcehut:~user/repo";
    let updated = uri
        .parse::<FlakeRef>()
        .unwrap()
        .with_ref(Some("v1.0.0".to_string()))
        .into_uri();

    assert_eq!(updated, "sourcehut:~user/repo/v1.0.0");
}

#[test]
fn test_set_ref_resource_emits_query_param() {
    // `Resource(Git)` has no path-component ref/rev shape in canonical
    // Nix's `git` scheme, so a ref written through the typed slot is
    // serialised as `?ref=<value>`.
    let uri = "git+https://github.com/example/repo";
    let updated = uri
        .parse::<FlakeRef>()
        .unwrap()
        .with_ref(Some("main".to_string()))
        .into_uri();

    assert_eq!(updated, "git+https://github.com/example/repo?ref=main");
}
