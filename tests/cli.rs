use insta::internals::Content;
use insta_cmd::{assert_cmd_snapshot, get_cargo_bin};
use rstest::rstest;
use std::fs;
use std::process::Command;

fn cli() -> Command {
    let mut cmd = Command::new(get_cargo_bin("flake-edit"));
    cmd.env("NO_COLOR", "1");
    cmd
}

fn fixture_path(name: &str) -> String {
    let dir = env!("CARGO_MANIFEST_DIR");
    format!("{dir}/tests/fixtures/{name}.flake.nix")
}

fn fixture_lock_path(name: &str) -> String {
    let dir = env!("CARGO_MANIFEST_DIR");
    format!("{dir}/tests/fixtures/{name}.flake.lock")
}

fn fixture_config_path(name: &str) -> String {
    let dir = env!("CARGO_MANIFEST_DIR");
    format!("{dir}/tests/fixtures/{name}.config.toml")
}

const FIXTURE_MARKER: &str = "/tests/fixtures/";

/// Add redaction to filter environment-dependent fixture paths in args metadata.
fn path_redactions(settings: &mut insta::Settings) {
    settings.add_dynamic_redaction(".args[]", |value, _path| {
        if let Some(s) = value.as_str()
            && let Some(idx) = s.find(FIXTURE_MARKER)
        {
            let rest = &s[idx + FIXTURE_MARKER.len()..];
            return Content::from(format!("[FIXTURES]/{rest}"));
        }
        value
    });
}

fn error_filters(settings: &mut insta::Settings) {
    settings.add_filter(r"\.rs:\d+", ".rs:<LINE>");
}

/// Filter fixture paths in stderr output (e.g., config parse errors).
fn stderr_path_filters(settings: &mut insta::Settings) {
    // Replace absolute paths containing /tests/fixtures/ with [FIXTURES]/
    settings.add_filter(r"'[^']*(/tests/fixtures/)([^']+)'", "'[FIXTURES]/$2'");
    // Also handle unquoted paths (e.g., in "Could not read lock file: /path/to/...")
    settings.add_filter(r"[^\s']*(/tests/fixtures/)([^\s]+)", "[FIXTURES]/$2");
}

#[rstest]
#[case("root")]
#[case("root_alt")]
#[case("toplevel_nesting")]
#[case("completely_flat_toplevel")]
#[case("completely_flat_toplevel_alt")]
#[case("one_level_nesting_flat")]
#[case("flat_nested_flat")]
#[case("first_nested_node")]
#[case("follows_cycle")]
fn test_list(#[case] fixture: &str) {
    let mut settings = insta::Settings::clone_current();
    path_redactions(&mut settings);
    settings.set_snapshot_suffix(fixture);
    settings.bind(|| {
        assert_cmd_snapshot!(cli().arg("--flake").arg(fixture_path(fixture)).arg("list"));
    });
}

#[rstest]
#[case("root", "simple")]
#[case("root", "toplevel")]
#[case("root", "json")]
#[case("root", "raw")]
fn test_list_format(#[case] fixture: &str, #[case] format: &str) {
    let mut settings = insta::Settings::clone_current();
    path_redactions(&mut settings);
    let suffix = format!("{fixture}_{format}");
    settings.set_snapshot_suffix(suffix);
    settings.bind(|| {
        assert_cmd_snapshot!(
            cli()
                .arg("--flake")
                .arg(fixture_path(fixture))
                .arg("list")
                .arg("--format")
                .arg(format)
        );
    });
}

#[rstest]
#[case("root", "vmsh", "github:mic92/vmsh")]
#[case("root_alt", "vmsh", "github:mic92/vmsh")]
#[case("toplevel_nesting", "vmsh", "github:mic92/vmsh")]
#[case("completely_flat_toplevel", "vmsh", "github:mic92/vmsh")]
#[case("completely_flat_toplevel_alt", "vmsh", "github:mic92/vmsh")]
#[case("one_level_nesting_flat", "vmsh", "github:mic92/vmsh")]
#[case("flat_nested_flat", "vmsh", "github:mic92/vmsh")]
#[case("first_nested_node", "vmsh", "github:mic92/vmsh")]
fn test_add(#[case] fixture: &str, #[case] id: &str, #[case] uri: &str) {
    let mut settings = insta::Settings::clone_current();
    path_redactions(&mut settings);
    let suffix = format!("{fixture}_{id}");
    settings.set_snapshot_suffix(suffix);
    settings.bind(|| {
        assert_cmd_snapshot!(
            cli()
                .arg("--flake")
                .arg(fixture_path(fixture))
                .arg("--diff")
                .arg("add")
                .arg(id)
                .arg(uri)
        );
    });
}

#[rstest]
#[case("root", "not_a_flake", "github:a-kenji/not_a_flake")]
fn test_add_no_flake(#[case] fixture: &str, #[case] id: &str, #[case] uri: &str) {
    let mut settings = insta::Settings::clone_current();
    path_redactions(&mut settings);
    let suffix = format!("{fixture}_{id}");
    settings.set_snapshot_suffix(suffix);
    settings.bind(|| {
        assert_cmd_snapshot!(
            cli()
                .arg("--flake")
                .arg(fixture_path(fixture))
                .arg("--diff")
                .arg("add")
                .arg("--no-flake")
                .arg(id)
                .arg(uri)
        );
    });
}

#[rstest]
#[case("root", "shallow_input", "github:foo/bar")]
fn test_add_shallow(#[case] fixture: &str, #[case] id: &str, #[case] uri: &str) {
    let mut settings = insta::Settings::clone_current();
    path_redactions(&mut settings);
    let suffix = format!("{fixture}_{id}");
    settings.set_snapshot_suffix(suffix);
    settings.bind(|| {
        assert_cmd_snapshot!(
            cli()
                .arg("--flake")
                .arg(fixture_path(fixture))
                .arg("--diff")
                .arg("add")
                .arg("--shallow")
                .arg(id)
                .arg(uri)
        );
    });
}

#[rstest]
#[case("root", "shallow_ref_input", "github:foo/bar", "main")]
fn test_add_shallow_with_ref(
    #[case] fixture: &str,
    #[case] id: &str,
    #[case] uri: &str,
    #[case] ref_or_rev: &str,
) {
    let mut settings = insta::Settings::clone_current();
    path_redactions(&mut settings);
    let suffix = format!("{fixture}_{id}");
    settings.set_snapshot_suffix(suffix);
    settings.bind(|| {
        assert_cmd_snapshot!(
            cli()
                .arg("--flake")
                .arg(fixture_path(fixture))
                .arg("--diff")
                .arg("add")
                .arg("--shallow")
                .arg("--ref-or-rev")
                .arg(ref_or_rev)
                .arg(id)
                .arg(uri)
        );
    });
}

#[rstest]
#[case("root")]
fn test_add_infer_id(#[case] fixture: &str) {
    let mut settings = insta::Settings::clone_current();
    path_redactions(&mut settings);
    settings.set_snapshot_suffix(fixture);
    settings.bind(|| {
        assert_cmd_snapshot!(
            cli()
                .arg("--flake")
                .arg(fixture_path(fixture))
                .arg("--diff")
                .arg("add")
                .arg("github:mic92/vmsh")
        );
    });
}

#[rstest]
#[case("root", "nixpkgs")]
#[case("root_alt", "nixpkgs")]
#[case("toplevel_nesting", "nixpkgs")]
#[case("completely_flat_toplevel", "nixpkgs")]
#[case("completely_flat_toplevel_alt", "nixpkgs")]
#[case("one_level_nesting_flat", "nixpkgs")]
#[case("flat_nested_flat", "nixpkgs")]
#[case("first_nested_node", "nixpkgs")]
#[case("root", "rust-overlay")]
fn test_remove(#[case] fixture: &str, #[case] id: &str) {
    let mut settings = insta::Settings::clone_current();
    path_redactions(&mut settings);
    let suffix = format!("{fixture}_{id}");
    settings.set_snapshot_suffix(suffix);
    settings.bind(|| {
        assert_cmd_snapshot!(
            cli()
                .arg("--flake")
                .arg(fixture_path(fixture))
                .arg("--diff")
                .arg("rm")
                .arg(id)
        );
    });
}

#[rstest]
#[case("root", "nixpkgs", "github:nixos/nixpkgs/nixos-24.05")]
#[case("root_alt", "nixpkgs", "github:nixos/nixpkgs/nixos-24.05")]
#[case("toplevel_nesting", "nixpkgs", "github:nixos/nixpkgs/nixos-24.05")]
#[case(
    "completely_flat_toplevel",
    "nixpkgs",
    "github:nixos/nixpkgs/nixos-24.05"
)]
#[case(
    "completely_flat_toplevel_alt",
    "nixpkgs",
    "github:nixos/nixpkgs/nixos-24.05"
)]
#[case(
    "one_level_nesting_flat",
    "nixpkgs",
    "github:nixos/nixpkgs/nixos-24.05"
)]
#[case("flat_nested_flat", "nixpkgs", "github:nixos/nixpkgs/nixos-24.05")]
#[case("first_nested_node", "nixpkgs", "github:nixos/nixpkgs/nixos-24.05")]
fn test_change(#[case] fixture: &str, #[case] id: &str, #[case] uri: &str) {
    let mut settings = insta::Settings::clone_current();
    path_redactions(&mut settings);
    let suffix = format!("{fixture}_{id}");
    settings.set_snapshot_suffix(suffix);
    settings.bind(|| {
        assert_cmd_snapshot!(
            cli()
                .arg("--flake")
                .arg(fixture_path(fixture))
                .arg("--diff")
                .arg("change")
                .arg(id)
                .arg(uri)
        );
    });
}

#[rstest]
#[case("root", "nixpkgs", "github:nixos/nixpkgs/nixos-24.05")]
fn test_change_shallow(#[case] fixture: &str, #[case] id: &str, #[case] uri: &str) {
    let mut settings = insta::Settings::clone_current();
    path_redactions(&mut settings);
    let suffix = format!("{fixture}_{id}");
    settings.set_snapshot_suffix(suffix);
    settings.bind(|| {
        assert_cmd_snapshot!(
            cli()
                .arg("--flake")
                .arg(fixture_path(fixture))
                .arg("--diff")
                .arg("change")
                .arg("--shallow")
                .arg(id)
                .arg(uri)
        );
    });
}

#[rstest]
#[case("root", "nonexistent-input")]
fn test_remove_nonexistent(#[case] fixture: &str, #[case] id: &str) {
    let mut settings = insta::Settings::clone_current();
    path_redactions(&mut settings);
    error_filters(&mut settings);
    let suffix = format!("{fixture}_{id}");
    settings.set_snapshot_suffix(suffix);
    settings.bind(|| {
        assert_cmd_snapshot!(
            cli()
                .arg("--flake")
                .arg(fixture_path(fixture))
                .arg("--diff")
                .arg("rm")
                .arg(id)
        );
    });
}

#[rstest]
#[case("root", "nonexistent-input", "github:foo/bar")]
fn test_change_nonexistent(#[case] fixture: &str, #[case] id: &str, #[case] uri: &str) {
    let mut settings = insta::Settings::clone_current();
    path_redactions(&mut settings);
    error_filters(&mut settings);
    let suffix = format!("{fixture}_{id}");
    settings.set_snapshot_suffix(suffix);
    settings.bind(|| {
        assert_cmd_snapshot!(
            cli()
                .arg("--flake")
                .arg(fixture_path(fixture))
                .arg("--diff")
                .arg("change")
                .arg(id)
                .arg(uri)
        );
    });
}

/// Test the follow command for nested-style inputs
#[rstest]
#[case("first_nested_node", "naersk.flake-utils", "flake-utils")]
#[case("root", "crane.flake-compat", "flake-compat")]
#[case("centerpiece", "home-manager.nixpkgs", "nixpkgs")]
#[case("centerpiece", "treefmt-nix.nixpkgs", "nixpkgs")]
#[case("mixed_style", "blueprint.nixpkgs", "nixpkgs")]
#[case("mixed_style", "blueprint.systems", "systems")]
#[case("mixed_style", "mprisd.nixpkgs", "nixpkgs")]
#[case("mixed_style", "mprisd.flake-parts", "flake-parts")]
fn test_add_follow(#[case] fixture: &str, #[case] input: &str, #[case] target: &str) {
    let mut settings = insta::Settings::clone_current();
    path_redactions(&mut settings);
    let suffix = format!("{fixture}_{}", input.replace('.', "_"));
    settings.set_snapshot_suffix(suffix);
    settings.bind(|| {
        assert_cmd_snapshot!(
            cli()
                .arg("--flake")
                .arg(fixture_path(fixture))
                .arg("--diff")
                .arg("add-follow")
                .arg(input)
                .arg(target)
        );
    });
}

/// Test the add-follow command for flat-style inputs
#[rstest]
#[case("one_level_nesting_flat", "rust-overlay.flake-compat", "flake-compat")]
#[case("mixed_style", "harmonia.nixpkgs", "nixpkgs")]
fn test_add_follow_flat(#[case] fixture: &str, #[case] input: &str, #[case] target: &str) {
    let mut settings = insta::Settings::clone_current();
    path_redactions(&mut settings);
    let suffix = format!("{fixture}_{}", input.replace('.', "_"));
    settings.set_snapshot_suffix(suffix);
    settings.bind(|| {
        assert_cmd_snapshot!(
            cli()
                .arg("--flake")
                .arg(fixture_path(fixture))
                .arg("--diff")
                .arg("add-follow")
                .arg(input)
                .arg(target)
        );
    });
}

/// Test the add-follow command with non-existent parent input
#[rstest]
#[case("root", "nonexistent.nixpkgs", "nixpkgs")]
fn test_add_follow_nonexistent(#[case] fixture: &str, #[case] input: &str, #[case] target: &str) {
    let mut settings = insta::Settings::clone_current();
    path_redactions(&mut settings);
    error_filters(&mut settings);
    let suffix = format!("{fixture}_{}", input.replace('.', "_"));
    settings.set_snapshot_suffix(suffix);
    settings.bind(|| {
        assert_cmd_snapshot!(
            cli()
                .arg("--flake")
                .arg(fixture_path(fixture))
                .arg("--diff")
                .arg("add-follow")
                .arg(input)
                .arg(target)
        );
    });
}

/// Test the follow command to automatically follow matching inputs
#[rstest]
#[case("centerpiece")] // Two nested nixpkgs inputs that can follow top-level nixpkgs
#[case("first_nested_node")] // naersk.nixpkgs already follows, utils.systems has no match
#[case("flat_nested_flat")] // poetry2nix follows already set, no other matches
#[case("root")] // Has follows in flake.nix but lockfile shows direct references
#[case("hyperconfig")] // Large real-world flake with mostly flat-style inputs
#[case("mixed_style")] // Mixed flat and nested inputs (harmonia flat, blueprint/mprisd nested)
#[case("follows_cycle")] // Cycle detection: treefmt-nix follows harmonia/treefmt-nix
#[case("stale_follows")] // Stale follows: crane.flake-compat no longer exists in lock
#[case("stale_follows_invalid_parent")] // Stale follows: nixpkgs.treefmt-nix doesn't exist
#[case("treefmt_transitive")] // treefmt has treefmt-nix as transitive input that matches top-level
fn test_follow(#[case] fixture: &str) {
    let mut settings = insta::Settings::clone_current();
    path_redactions(&mut settings);
    settings.set_snapshot_suffix(fixture);
    settings.bind(|| {
        assert_cmd_snapshot!(
            cli()
                .arg("--flake")
                .arg(fixture_path(fixture))
                .arg("--lock-file")
                .arg(fixture_lock_path(fixture))
                .arg("--diff")
                .arg("follow")
        );
    });
}

/// Test the follow command with a custom config file
#[rstest]
#[case("centerpiece", "ignore_treefmt")] // Config ignores treefmt-nix.nixpkgs, only home-manager follows
fn test_follow_with_config(#[case] fixture: &str, #[case] config: &str) {
    let mut settings = insta::Settings::clone_current();
    path_redactions(&mut settings);
    let suffix = format!("{fixture}_{config}");
    settings.set_snapshot_suffix(suffix);
    settings.bind(|| {
        assert_cmd_snapshot!(
            cli()
                .arg("--flake")
                .arg(fixture_path(fixture))
                .arg("--lock-file")
                .arg(fixture_lock_path(fixture))
                .arg("--config")
                .arg(fixture_config_path(config))
                .arg("--diff")
                .arg("follow")
        );
    });
}

/// Test behavior with a malformed config file (returns error with line info)
#[rstest]
#[case("centerpiece", "malformed")] // Malformed TOML shows parse error with line number
fn test_follow_with_malformed_config(#[case] fixture: &str, #[case] config: &str) {
    let mut settings = insta::Settings::clone_current();
    path_redactions(&mut settings);
    stderr_path_filters(&mut settings);
    let suffix = format!("{fixture}_{config}");
    settings.set_snapshot_suffix(suffix);
    settings.bind(|| {
        assert_cmd_snapshot!(
            cli()
                .arg("--flake")
                .arg(fixture_path(fixture))
                .arg("--lock-file")
                .arg(fixture_lock_path(fixture))
                .arg("--config")
                .arg(fixture_config_path(config))
                .arg("--diff")
                .arg("follow")
        );
    });
}

/// Test that --flake and --lock are incompatible with follow [paths]
#[test]
fn test_follow_paths_incompatible_with_flake_flag() {
    let mut settings = insta::Settings::clone_current();
    path_redactions(&mut settings);
    settings.bind(|| {
        assert_cmd_snapshot!(
            cli()
                .arg("--flake")
                .arg(fixture_path("centerpiece"))
                .arg("follow")
                .arg(fixture_path("first_nested_node"))
        );
    });
}

#[test]
fn test_follow_paths_incompatible_with_lock_flag() {
    let mut settings = insta::Settings::clone_current();
    path_redactions(&mut settings);
    settings.bind(|| {
        assert_cmd_snapshot!(
            cli()
                .arg("--lock-file")
                .arg(fixture_lock_path("centerpiece"))
                .arg("follow")
                .arg(fixture_path("first_nested_node"))
        );
    });
}

/// Test follow with positional path (single file)
#[rstest]
#[case("centerpiece")] // Single file with nested nixpkgs inputs
fn test_follow_paths_single(#[case] fixture: &str) {
    let mut settings = insta::Settings::clone_current();
    path_redactions(&mut settings);
    stderr_path_filters(&mut settings);
    settings.set_snapshot_suffix(fixture);
    settings.bind(|| {
        assert_cmd_snapshot!(cli().arg("--diff").arg("follow").arg(fixture_path(fixture)));
    });
}

/// Test follow with multiple positional paths (batch mode)
#[rstest]
fn test_follow_paths_batch() {
    let mut settings = insta::Settings::clone_current();
    path_redactions(&mut settings);
    stderr_path_filters(&mut settings);
    settings.bind(|| {
        assert_cmd_snapshot!(
            cli()
                .arg("--diff")
                .arg("follow")
                .arg(fixture_path("centerpiece"))
                .arg(fixture_path("first_nested_node"))
        );
    });
}

/// Helper to copy a fixture (flake.nix and flake.lock) to a target directory.
fn copy_fixture_to_dir(fixture_name: &str, target_dir: &std::path::Path) {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let flake_src = format!("{}/tests/fixtures/{}.flake.nix", manifest_dir, fixture_name);
    let lock_src = format!(
        "{}/tests/fixtures/{}.flake.lock",
        manifest_dir, fixture_name
    );

    fs::copy(&flake_src, target_dir.join("flake.nix")).expect("Failed to copy flake.nix");
    fs::copy(&lock_src, target_dir.join("flake.lock")).expect("Failed to copy flake.lock");
}

/// Integration test for follow with real directory structure.
///
/// Creates a tmpdir with multiple flake directories:
/// ```
/// tmpdir/
///   flake.nix + flake.lock (centerpiece)
///   other/
///     flake.nix + flake.lock (mixed_style)
///   another/
///     flake.nix + flake.lock (first_nested_node)
/// ```
///
/// Then runs `follow` on all three and snapshots the resulting flake.nix files.
#[test]
fn test_follow_multi_directory() {
    let tmpdir = tempfile::tempdir().expect("Failed to create tmpdir");
    let root = tmpdir.path();

    // Create directory structure
    let other_dir = root.join("other");
    let another_dir = root.join("another");
    fs::create_dir(&other_dir).expect("Failed to create other/");
    fs::create_dir(&another_dir).expect("Failed to create another/");

    // Copy fixtures
    copy_fixture_to_dir("centerpiece", root);
    copy_fixture_to_dir("mixed_style", &other_dir);
    copy_fixture_to_dir("first_nested_node", &another_dir);

    // Run follow on all three flake.nix files
    let output = Command::new(get_cargo_bin("flake-edit"))
        .env("NO_COLOR", "1")
        .arg("follow")
        .arg(root.join("flake.nix"))
        .arg(other_dir.join("flake.nix"))
        .arg(another_dir.join("flake.nix"))
        .output()
        .expect("Failed to run flake-edit");

    assert!(
        output.status.success(),
        "flake-edit failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Read the resulting flake.nix files
    let root_result =
        fs::read_to_string(root.join("flake.nix")).expect("Failed to read root flake.nix");
    let other_result =
        fs::read_to_string(other_dir.join("flake.nix")).expect("Failed to read other/flake.nix");
    let another_result = fs::read_to_string(another_dir.join("flake.nix"))
        .expect("Failed to read another/flake.nix");

    // Snapshot all results together
    insta::assert_snapshot!("multi_directory_root", root_result);
    insta::assert_snapshot!("multi_directory_other", other_result);
    insta::assert_snapshot!("multi_directory_another", another_result);
}

/// Test follow without arguments (runs on current directory).
///
/// Creates a tmpdir with flake.nix + flake.lock, changes to that directory,
/// and runs `flake-edit follow --diff` without any path arguments.
#[test]
fn test_follow_current_directory() {
    let tmpdir = tempfile::tempdir().expect("Failed to create tmpdir");
    let root = tmpdir.path();

    // Copy fixture (centerpiece has home-manager.nixpkgs and treefmt-nix.nixpkgs to follow)
    copy_fixture_to_dir("centerpiece", root);

    // Run follow --diff without path arguments, using current_dir
    let mut settings = insta::Settings::clone_current();
    settings.set_snapshot_suffix("centerpiece");
    settings.bind(|| {
        assert_cmd_snapshot!(
            Command::new(get_cargo_bin("flake-edit"))
                .env("NO_COLOR", "1")
                .current_dir(root)
                .arg("--diff")
                .arg("follow")
        );
    });
}
