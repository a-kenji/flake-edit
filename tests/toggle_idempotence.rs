//! `flake-edit toggle` must round-trip: toggling twice from any state
//! that toggle produced restores the file byte for byte. The one
//! permitted normalization is a hand-written `#x` comment becoming
//! `# x` after its first round trip.

#![cfg(feature = "application")]

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

fn run_toggle(flake: &Path, args: &[&str], label: &str) -> String {
    let mut cmd = cli();
    cmd.arg("--flake").arg(flake).arg("--no-lock").arg("toggle");
    cmd.args(args);
    let output = cmd.output().unwrap_or_else(|e| panic!("{label}: {e}"));
    assert!(
        output.status.success(),
        "{label} failed (status {:?}): stdout={}\nstderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Toggle the fixture's single toggleable input twice and assert the file
/// is restored byte for byte.
#[rstest]
#[case("toggle_flat")]
#[case("toggle_toplevel_flat")]
fn toggle_twice_restores_file(#[case] fixture: &str) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let flake = tmp.path().join("flake.nix");
    fs::copy(fixture_path(fixture), &flake).expect("copy flake.nix");
    let original = fs::read_to_string(&flake).expect("read original");

    run_toggle(&flake, &[], "first toggle");
    let after_first = fs::read_to_string(&flake).expect("read after first");
    assert_ne!(after_first, original, "first toggle must change the file");

    run_toggle(&flake, &[], "second toggle");
    let after_second = fs::read_to_string(&flake).expect("read after second");
    assert_eq!(
        after_second, original,
        "fixture {fixture}: double toggle must restore the file byte for byte",
    );
}

/// A hand-written `#crane.url = ...` (no space after the marker) is
/// normalized to `# crane.url = ...` by its first round trip. Further
/// round trips are byte-stable.
#[test]
fn no_space_marker_normalizes_once_then_round_trips() {
    let content = r#"{
  inputs = {
    #crane.url = "github:a-kenji/crane";
    crane.url = "github:ipetkov/crane";
  };
  outputs = _: { };
}
"#;
    let tmp = tempfile::tempdir().expect("tempdir");
    let flake = tmp.path().join("flake.nix");
    fs::write(&flake, content).expect("write flake.nix");

    run_toggle(&flake, &[], "first toggle");
    run_toggle(&flake, &[], "second toggle");
    let normalized = fs::read_to_string(&flake).expect("read normalized");
    assert_eq!(
        normalized,
        content.replace("#crane.url", "# crane.url"),
        "the only permitted normalization is `#x` -> `# x`",
    );

    run_toggle(&flake, &[], "third toggle");
    run_toggle(&flake, &[], "fourth toggle");
    let stable = fs::read_to_string(&flake).expect("read after fourth");
    assert_eq!(stable, normalized, "later round trips must be byte-stable");
}

/// First use of a new ref stores it as an alternate. From then on the
/// zero-argument flip oscillates between the two stored variants.
#[test]
fn synthesis_then_flips_reach_a_two_state_cycle() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let flake = tmp.path().join("flake.nix");
    fs::copy(fixture_path("toggle_block"), &flake).expect("copy flake.nix");

    run_toggle(
        &flake,
        &["rust-overlay", "github:a-kenji/rust-overlay"],
        "synthesis",
    );
    let forked = fs::read_to_string(&flake).expect("read after synthesis");
    assert!(forked.contains(r#"# url = "github:oxalica/rust-overlay";"#));
    assert!(forked.contains(r#"url = "github:a-kenji/rust-overlay";"#));

    run_toggle(&flake, &[], "flip back");
    let upstream = fs::read_to_string(&flake).expect("read after flip back");
    assert!(upstream.contains(r#"url = "github:oxalica/rust-overlay";"#));
    assert!(
        upstream.contains(r#"# url = "github:a-kenji/rust-overlay";"#),
        "flipping back must keep the alternate stored in the file, got:\n{upstream}",
    );

    run_toggle(&flake, &[], "flip forward again");
    let forked_again = fs::read_to_string(&flake).expect("read after third flip");
    assert_eq!(
        forked_again, forked,
        "toggling twice from a toggle-produced state must be byte-identical",
    );
}

/// `--remove` through the active url undoes a first use: storing a new
/// ref and then removing it restores the file byte for byte.
#[test]
fn synthesize_then_remove_restores_file() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let flake = tmp.path().join("flake.nix");
    fs::copy(fixture_path("toggle_block"), &flake).expect("copy flake.nix");
    let original = fs::read_to_string(&flake).expect("read original");

    run_toggle(
        &flake,
        &["rust-overlay", "github:a-kenji/rust-overlay"],
        "synthesize",
    );
    let synthesized = fs::read_to_string(&flake).expect("read after synthesis");
    assert_ne!(synthesized, original, "synthesis must change the file");

    run_toggle(
        &flake,
        &["--remove", "github:a-kenji/rust-overlay"],
        "remove",
    );
    let restored = fs::read_to_string(&flake).expect("read after remove");
    assert_eq!(
        restored, original,
        "removing the synthesized variant must restore the file byte for byte",
    );
}

/// A bare input id discards the active url and reverts to the stored
/// alternate. It is the destructive sibling of the plain flip. Storing a
/// new ref and then removing by id restores the file byte for byte.
#[test]
fn store_new_ref_then_remove_by_id_restores_file() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let flake = tmp.path().join("flake.nix");
    fs::copy(fixture_path("toggle_block"), &flake).expect("copy flake.nix");
    let original = fs::read_to_string(&flake).expect("read original");

    run_toggle(
        &flake,
        &["rust-overlay", "github:a-kenji/rust-overlay"],
        "store new ref",
    );
    let after_store = fs::read_to_string(&flake).expect("read after storing the ref");
    assert_ne!(
        after_store, original,
        "storing a new ref must change the file"
    );

    run_toggle(&flake, &["--remove", "rust-overlay"], "remove by id");
    let restored = fs::read_to_string(&flake).expect("read after remove");
    assert_eq!(
        restored, original,
        "removing by bare id must discard the active url and revert to the alternate byte for byte",
    );
}

/// The no-argument `--remove` resolves to the sole toggleable input and
/// acts on its active url exactly like the bare-id form. It discards the
/// active url and reverts to the stored alternate.
#[test]
fn store_new_ref_then_remove_no_args_restores_file() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let flake = tmp.path().join("flake.nix");
    fs::copy(fixture_path("toggle_block"), &flake).expect("copy flake.nix");
    let original = fs::read_to_string(&flake).expect("read original");

    run_toggle(
        &flake,
        &["rust-overlay", "github:a-kenji/rust-overlay"],
        "store new ref",
    );
    let after_store = fs::read_to_string(&flake).expect("read after storing the ref");
    assert_ne!(
        after_store, original,
        "storing a new ref must change the file"
    );

    run_toggle(&flake, &["--remove"], "remove no args");
    let restored = fs::read_to_string(&flake).expect("read after remove");
    assert_eq!(
        restored, original,
        "the no-arg remove must discard the active url and revert byte for byte",
    );
}

/// Removing a commented alternate cannot change the resolved source, so
/// the lockfile refresh is skipped. Removing the active url flips first
/// and refreshes the lock like any other toggle. A stub `nix` on PATH
/// stands in for the real lock run.
#[test]
fn remove_refreshes_lock_only_when_the_source_changes() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().expect("tempdir");
    let bin = tmp.path().join("bin");
    fs::create_dir(&bin).expect("create shim dir");
    let shim = bin.join("nix");
    fs::write(&shim, "#!/bin/sh\nexit 0\n").expect("write nix shim");
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).expect("chmod shim");
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let run_remove = |flake: &Path, reference: &str, label: &str| -> String {
        let output = cli()
            .arg("--flake")
            .arg(flake)
            .arg("toggle")
            .arg("--remove")
            .arg(reference)
            .env("PATH", &path)
            .output()
            .unwrap_or_else(|e| panic!("{label}: {e}"));
        assert!(
            output.status.success(),
            "{label} failed: {}",
            String::from_utf8_lossy(&output.stderr),
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    };

    let flake = tmp.path().join("flake.nix");
    fs::copy(fixture_path("toggle_flat"), &flake).expect("copy flake.nix");
    let stdout = run_remove(&flake, "github:a-kenji/rust-overlay", "remove alternate");
    assert_eq!(
        stdout.lines().collect::<Vec<_>>(),
        vec!["Removed rust-overlay alternate: github:a-kenji/rust-overlay"],
        "a comment-only removal must not refresh the lock",
    );

    fs::copy(fixture_path("toggle_flat"), &flake).expect("reset flake.nix");
    let stdout = run_remove(&flake, "github:oxalica/rust-overlay", "remove active");
    assert_eq!(
        stdout.lines().collect::<Vec<_>>(),
        vec![
            "Updated flake.lock",
            "Toggled rust-overlay: github:oxalica/rust-overlay -> github:a-kenji/rust-overlay",
            "Removed rust-overlay alternate: github:oxalica/rust-overlay",
        ],
        "removing the active url flips first and refreshes the lock",
    );
}

/// The lockfile refresh line prints before the success line, as in every
/// other subcommand. A stub `nix` on PATH stands in for the real lock
/// run.
#[test]
fn lock_line_prints_before_success_line() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().expect("tempdir");
    let flake = tmp.path().join("flake.nix");
    fs::copy(fixture_path("toggle_flat"), &flake).expect("copy flake.nix");

    let bin = tmp.path().join("bin");
    fs::create_dir(&bin).expect("create shim dir");
    let shim = bin.join("nix");
    fs::write(&shim, "#!/bin/sh\nexit 0\n").expect("write nix shim");
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).expect("chmod shim");

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let output = cli()
        .arg("--flake")
        .arg(&flake)
        .arg("toggle")
        .env("PATH", path)
        .output()
        .expect("run toggle with stub nix");
    assert!(output.status.success(), "toggle must succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines,
        vec![
            "Updated flake.lock",
            "Toggled rust-overlay: github:oxalica/rust-overlay -> github:a-kenji/rust-overlay",
        ],
        "the lock line prints first, then the success line",
    );
}

/// `--diff` prints the patch and writes nothing.
#[test]
fn diff_mode_does_not_write() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let flake = tmp.path().join("flake.nix");
    fs::copy(fixture_path("toggle_flat"), &flake).expect("copy flake.nix");
    let original = fs::read_to_string(&flake).expect("read original");

    let output = cli()
        .arg("--flake")
        .arg(&flake)
        .arg("--diff")
        .arg("toggle")
        .output()
        .expect("run toggle --diff");
    assert!(output.status.success(), "toggle --diff must succeed");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("+++ modified"),
        "the diff must be printed",
    );
    assert_eq!(
        fs::read_to_string(&flake).expect("read after diff"),
        original,
        "diff mode must not write",
    );
}
