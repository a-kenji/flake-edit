use insta_cmd::{assert_cmd_snapshot, get_cargo_bin};
use rstest::rstest;
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

fn error_filters(settings: &mut insta::Settings) {
    settings.add_filter(r"\.rs:\d+", ".rs:<LINE>");
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
fn test_list(#[case] fixture: &str) {
    insta::with_settings!({
        snapshot_suffix => fixture
    }, {
        assert_cmd_snapshot!(cli().arg("--flake").arg(fixture_path(fixture)).arg("list"));
    });
}

#[rstest]
#[case("root", "simple")]
#[case("root", "toplevel")]
#[case("root", "json")]
#[case("root", "raw")]
fn test_list_format(#[case] fixture: &str, #[case] format: &str) {
    let suffix = format!("{fixture}_{format}");
    insta::with_settings!({
        snapshot_suffix => suffix
    }, {
        assert_cmd_snapshot!(cli()
            .arg("--flake")
            .arg(fixture_path(fixture))
            .arg("list")
            .arg("--format")
            .arg(format));
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
    let suffix = format!("{fixture}_{id}");
    insta::with_settings!({
        snapshot_suffix => suffix
    }, {
        assert_cmd_snapshot!(cli()
            .arg("--flake")
            .arg(fixture_path(fixture))
            .arg("--diff")
            .arg("add")
            .arg(id)
            .arg(uri));
    });
}

#[rstest]
#[case("root", "not_a_flake", "github:a-kenji/not_a_flake")]
fn test_add_no_flake(#[case] fixture: &str, #[case] id: &str, #[case] uri: &str) {
    let suffix = format!("{fixture}_{id}");
    insta::with_settings!({
        snapshot_suffix => suffix
    }, {
        assert_cmd_snapshot!(cli()
            .arg("--flake")
            .arg(fixture_path(fixture))
            .arg("--diff")
            .arg("add")
            .arg("--no-flake")
            .arg(id)
            .arg(uri));
    });
}

#[rstest]
#[case("root")]
fn test_add_infer_id(#[case] fixture: &str) {
    insta::with_settings!({
        snapshot_suffix => fixture
    }, {
        assert_cmd_snapshot!(cli()
            .arg("--flake")
            .arg(fixture_path(fixture))
            .arg("--diff")
            .arg("add")
            .arg("github:mic92/vmsh"));
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
fn test_remove(#[case] fixture: &str, #[case] id: &str) {
    let suffix = format!("{fixture}_{id}");
    insta::with_settings!({
        snapshot_suffix => suffix
    }, {
        assert_cmd_snapshot!(cli()
            .arg("--flake")
            .arg(fixture_path(fixture))
            .arg("--diff")
            .arg("rm")
            .arg(id));
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
    let suffix = format!("{fixture}_{id}");
    insta::with_settings!({
        snapshot_suffix => suffix
    }, {
        assert_cmd_snapshot!(cli()
            .arg("--flake")
            .arg(fixture_path(fixture))
            .arg("--diff")
            .arg("change")
            .arg(id)
            .arg(uri));
    });
}

#[rstest]
#[case("root", "nonexistent-input")]
fn test_remove_nonexistent(#[case] fixture: &str, #[case] id: &str) {
    let mut settings = insta::Settings::clone_current();
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
