//! `flake-edit follow` must reach a fixed point in a single invocation: a
//! second run on the resulting `flake.nix` produces no further changes.

use insta_cmd::get_cargo_bin;
use rstest::rstest;
use std::fs;
use std::path::Path;
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

/// Run `flake-edit follow` against a copy of the fixture, then run it again
/// against the resulting `flake.nix`. Assert the second run made no changes.
fn assert_idempotent(fixture: &str, config: Option<&str>) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let flake_dest = tmp.path().join("flake.nix");
    let lock_dest = tmp.path().join("flake.lock");
    fs::copy(fixture_path(fixture), &flake_dest).expect("copy flake.nix");
    fs::copy(fixture_lock_path(fixture), &lock_dest).expect("copy flake.lock");

    run_follow(&flake_dest, &lock_dest, config, "first run");

    let after_first = fs::read_to_string(&flake_dest).expect("read after first run");

    run_follow(&flake_dest, &lock_dest, config, "second run");

    let after_second = fs::read_to_string(&flake_dest).expect("read after second run");

    assert_eq!(
        after_first, after_second,
        "fixture {fixture} is not idempotent: second run mutated the flake.nix",
    );
}

fn run_follow(flake: &Path, lock: &Path, config: Option<&str>, label: &str) {
    let mut cmd = cli();
    cmd.arg("--flake").arg(flake).arg("--lock-file").arg(lock);
    if let Some(c) = config {
        cmd.arg("--config").arg(fixture_config_path(c));
    }
    cmd.arg("follow");
    let output = cmd.output().unwrap_or_else(|e| panic!("{label}: {e}"));
    assert!(
        output.status.success(),
        "{label} failed (status {:?}): stdout={}\nstderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[rstest]
#[case("transitive_self_named", None)] // self-named nested-path round-trip
#[case(
    "transitive_promote_unlocks_deeper",
    Some("transitive_promote_unlocks_deeper")
)] // top-level promotion plus a deeper follow that depends on it
#[case("transitive_grandchild", Some("deep_follows_2"))] // depth-2 emission
#[case("stale_edge_unblocks_follow", Some("stale_edge_unblocks_follow"))] // stale-edge removal that simultaneously unblocks a follow candidate
#[case("follows_empty_target", None)] // empty `follows = ""` must not oscillate
#[case("follow_respects_nulled", None)] // nulled `follows = ""` must be respected
#[case("follow_respects_ancestor", None)] // ancestor-declared subtree must not be overridden
#[case("nulled_stale_source_removed", None)]
#[case("nulled_stale_depth3", None)]
#[case(
    "prune_empty_intermediate_inputs",
    Some("prune_empty_intermediate_inputs")
)] // empty intermediate `inputs = { ... }` block prune must converge
fn follow_is_idempotent(#[case] fixture: &str, #[case] config: Option<&str>) {
    assert_idempotent(fixture, config);
}
