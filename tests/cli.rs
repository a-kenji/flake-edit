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
#[case("deeply_nested_inputs")]
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
#[case("comments_before_brace", "vmsh", "github:mic92/vmsh")]
#[case("all_blanks", "vmsh", "github:mic92/vmsh")]
#[case("deeply_nested_inputs", "vmsh", "github:mic92/vmsh")]
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
#[case("all_blanks", "not_a_flake", "github:a-kenji/not_a_flake")]
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
#[case("comments_before_brace", "nixpkgs")]
#[case("deeply_nested_inputs", "nixpkgs")]
#[case("root", "rust-overlay")]
#[case("outputs_leading_comma_remove_first", "nixpkgs-unstable")]
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
#[case("deeply_nested_inputs", "nixpkgs", "github:nixos/nixpkgs/nixos-24.05")]
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
#[case("quoted_dotted_nested", "\"lib-v1.5\".nixpkgs", "nixpkgs")]
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
#[case("flat_toplevel_no_follows", "crane.nixpkgs", "nixpkgs")]
#[case("flat_toplevel_no_follows", "fenix.nixpkgs", "nixpkgs")]
#[case("flat_toplevel_comments", "crane.nixpkgs", "nixpkgs")]
#[case("flat_toplevel_comments", "fenix.nixpkgs", "nixpkgs")]
#[case("toplevel_block_nested_follows", "blocky.flake-parts", "flake-parts")]
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

/// Test add-follow when follows already exists (should be no-op for same target)
#[rstest]
#[case("existing_follows_nested", "rust-overlay.nixpkgs", "nixpkgs")]
#[case("existing_follows_nested", "devenv.nixpkgs", "nixpkgs")]
#[case("existing_follows_flat", "rust-overlay.nixpkgs", "nixpkgs")]
#[case("existing_follows_flat", "naersk.nixpkgs", "nixpkgs")]
#[case("completely_flat_toplevel", "crane.nixpkgs", "nixpkgs")]
#[case("completely_flat_toplevel", "rust-overlay.nixpkgs", "nixpkgs")]
fn test_add_follow_existing_same(#[case] fixture: &str, #[case] input: &str, #[case] target: &str) {
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

/// Test add-follow when follows exists with different target (should retarget)
#[rstest]
#[case("existing_follows_nested_retarget", "rust-overlay.nixpkgs", "nixpkgs")]
#[case("existing_follows_flat_retarget", "rust-overlay.nixpkgs", "nixpkgs")]
#[case("flat_toplevel_existing_follows", "crane.nixpkgs", "nixpkgs")]
fn test_add_follow_retarget(#[case] fixture: &str, #[case] input: &str, #[case] target: &str) {
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

/// Test add-follow on input that already has a *different* follows (should add new one)
#[rstest]
#[case("existing_follows_mixed", "devenv.flake-utils", "flake-utils")]
fn test_add_follow_existing_different_input(
    #[case] fixture: &str,
    #[case] input: &str,
    #[case] target: &str,
) {
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

/// `add-follow` with a 3+ segment dot path (e.g. `a.b.c`) used to silently
/// emit malformed Nix because only the first `.` was treated as the
/// `inputs.` separator. Reject these up front with a depth error.
#[test]
fn add_follow_rejects_three_segment_dot_path() {
    let mut settings = insta::Settings::clone_current();
    path_redactions(&mut settings);
    error_filters(&mut settings);
    let output = cli()
        .arg("--flake")
        .arg(fixture_path("root"))
        .arg("--diff")
        .arg("add-follow")
        .arg("neovim.nixvim.flake-parts")
        .arg("flake-parts")
        .output()
        .expect("run flake-edit");
    assert!(!output.status.success(), "expected non-zero exit");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stderr.contains("depth") || stderr.contains("3 segments"),
        "stderr should mention depth or segment count, got:\n{stderr}",
    );
    settings.bind(|| {
        assert_cmd_snapshot!(
            cli()
                .arg("--flake")
                .arg(fixture_path("root"))
                .arg("--diff")
                .arg("add-follow")
                .arg("neovim.nixvim.flake-parts")
                .arg("flake-parts")
        );
    });
}

/// Slash-separated input paths are not a recognized syntax: the whole string
/// becomes a single segment and the existing `Input not found` error must
/// remain intact (regression guard against accidentally widening the parser).
#[test]
fn add_follow_rejects_slash_form_unrecognized() {
    let mut settings = insta::Settings::clone_current();
    path_redactions(&mut settings);
    error_filters(&mut settings);
    let output = cli()
        .arg("--flake")
        .arg(fixture_path("root"))
        .arg("--diff")
        .arg("add-follow")
        .arg("neovim/nixvim/flake-parts")
        .arg("flake-parts")
        .output()
        .expect("run flake-edit");
    assert!(!output.status.success(), "expected non-zero exit");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stderr.contains("not found"),
        "slash-form must hit the existing 'not found' error, got:\n{stderr}",
    );
    settings.bind(|| {
        assert_cmd_snapshot!(
            cli()
                .arg("--flake")
                .arg(fixture_path("root"))
                .arg("--diff")
                .arg("add-follow")
                .arg("neovim/nixvim/flake-parts")
                .arg("flake-parts")
        );
    });
}

#[test]
fn add_follow_accepts_two_segment_dot_path_unchanged() {
    let mut settings = insta::Settings::clone_current();
    path_redactions(&mut settings);
    settings.bind(|| {
        assert_cmd_snapshot!(
            cli()
                .arg("--flake")
                .arg(fixture_path("root"))
                .arg("--diff")
                .arg("add-follow")
                .arg("rust-overlay.flake-compat")
                .arg("flake-utils")
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
#[case("toplevel_block_nested_follows")] // Top-level block-style input should accept new nested follows
#[case("follows_cycle")] // Cycle detection: treefmt-nix follows harmonia/treefmt-nix
#[case("stale_follows")] // Stale follows: crane.flake-compat no longer exists in lock
#[case("stale_follows_invalid_parent")] // Stale follows: nixpkgs.treefmt-nix doesn't exist
#[case("treefmt_transitive")] // treefmt has treefmt-nix as transitive input that matches top-level
#[case("multi_hop_cycle")] // multi-hop a -> b -> c declared chain
#[case("dot_ancestor_cycle")] // dot-named participant in a multi-hop chain
#[case("lockfile_only_cycle")] // cycle closes only through resolved lockfile edges
#[case("stale_lockfile_only")] // stale-edge detection alongside the resolver
#[case("split_inputs_block_and_flat")] // some inputs in a block, neovim flat outside
#[case("stale_lock")] // declared follows the lockfile didn't apply
#[case("transitive_grandchild")] // baseline: depth-2 candidate, default max_depth=1 ignores it
#[case("transitive_grandchild_existing")] // baseline: handwritten depth-2 follows, no-op
#[case("transitive_grandchild_cycle")] // baseline: depth-2 candidate skipped (cycle)
#[case("transitive_self_named")] // self-named depth-3 follows must round-trip without removal
#[case("follow_slash_syntax")] // slash-form `follows = "parent/child"` is the alias of `parent.child`
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
#[case("treefmt_transitive", "transitive")] // Transitive follows with transitive_min = 2
#[case("transitive_grandchild", "deep_follows_2")] // max_depth=2 emits depth-2 follows
#[case("transitive_grandchild_existing", "deep_follows_2")] // handwritten depth-2 already present, no-op
#[case("transitive_grandchild_cycle", "deep_follows_2")] // depth-2 candidate skipped due to cycle
#[case("depth_upstream_redundant", "depth_upstream_redundant")] // upstream propagation makes depth-2 follow redundant; nothing emitted
#[case("depth_upstream_partial", "depth_upstream_partial")] // partial upstream coverage: nixpkgs skipped, flake-utils emitted
#[case(
    "depth_upstream_redundant_declared",
    "depth_upstream_redundant_declared"
)] // already-declared depth-2 follow auto-removed
#[case("depth_upstream_redundant_partial", "depth_upstream_redundant_partial")] // mixed: nixpkgs depth-2 removed, flake-utils kept
#[case("depth_upstream_redundant_depth3", "depth_upstream_redundant_depth3")] // depth-3 chain auto-removed
#[case(
    "transitive_promote_with_upstream_redundant",
    "transitive_promote_with_upstream_redundant"
)]
// transitive_min=2 promotes rust-analyzer-src while sibling depth-2 nixpkgs follow stays suppressed by upstream propagation
#[case(
    "transitive_promote_unlocks_deeper",
    "transitive_promote_unlocks_deeper"
)]
// promoting flake-compat to a new top-level unlocks a deeper nested follow with the same canonical name in the same invocation
#[case("stale_edge_unblocks_follow", "stale_edge_unblocks_follow")]
// a stale follows declaration would block a valid candidate via the cycle check; the removal and the unblocked follow must land in one invocation
#[case("prune_empty_intermediate_inputs", "prune_empty_intermediate_inputs")]
// depth-2 redundant follow is the sole entry of an `inputs = { ... }` block; auto-removal must collapse the now-empty intermediate block
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

/// Test the follow command with --transitive flag (overrides config)
#[rstest]
#[case("treefmt_transitive", 2)] // Same as config-based test but via CLI flag
fn test_follow_with_transitive_flag(#[case] fixture: &str, #[case] min: usize) {
    let mut settings = insta::Settings::clone_current();
    path_redactions(&mut settings);
    let suffix = format!("{fixture}_{min}");
    settings.set_snapshot_suffix(suffix);
    settings.bind(|| {
        assert_cmd_snapshot!(
            cli()
                .arg("--flake")
                .arg(fixture_path(fixture))
                .arg("--lock-file")
                .arg(fixture_lock_path(fixture))
                .arg("--diff")
                .arg("follow")
                .arg("--transitive")
                .arg(min.to_string())
        );
    });
}

/// Test the follow command with --transitive flag without explicit value (defaults to 2)
#[rstest]
#[case("treefmt_transitive")]
fn test_follow_with_transitive_flag_default(#[case] fixture: &str) {
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
                .arg("--transitive")
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

/// Lockfiles whose `inputs` map contains empty `[]` arrays (a
/// follows declaration whose chain has been overridden away) must
/// deserialize and the `follow` subcommand must run to completion.
#[rstest]
fn test_follow_empty_indirect_lockfile() {
    let mut settings = insta::Settings::clone_current();
    path_redactions(&mut settings);
    stderr_path_filters(&mut settings);
    settings.bind(|| {
        assert_cmd_snapshot!(
            cli()
                .arg("--flake")
                .arg(fixture_path("lock_indirect_empty"))
                .arg("--lock-file")
                .arg(fixture_lock_path("lock_indirect_empty"))
                .arg("--diff")
                .arg("follow")
                .arg("--depth")
                .arg("2")
        );
    });
}

/// `inputs.X.follows = ""` is a real, intentional Nix idiom for
/// "this nested input has no follows / pinning override". `list`
/// must surface it as `=> ""` and `follow` must not synthesize a
/// bogus self-referential edge from it.
#[rstest]
fn test_list_empty_target_follows() {
    let mut settings = insta::Settings::clone_current();
    path_redactions(&mut settings);
    stderr_path_filters(&mut settings);
    settings.bind(|| {
        assert_cmd_snapshot!(
            cli()
                .arg("--flake")
                .arg(fixture_path("follows_empty_target"))
                .arg("--lock-file")
                .arg(fixture_lock_path("follows_empty_target"))
                .arg("list")
        );
    });
}

#[rstest]
fn test_follow_empty_target_follows() {
    let mut settings = insta::Settings::clone_current();
    path_redactions(&mut settings);
    stderr_path_filters(&mut settings);
    settings.bind(|| {
        assert_cmd_snapshot!(
            cli()
                .arg("--flake")
                .arg(fixture_path("follows_empty_target"))
                .arg("--lock-file")
                .arg(fixture_lock_path("follows_empty_target"))
                .arg("--diff")
                .arg("follow")
                .arg("--depth")
                .arg("2")
        );
    });
}

/// When the user has declared a parent override like
/// `clan-core.inputs.treefmt-nix.follows = "systems"`, the auto-deduplicator
/// must not propose a deeper override
/// `clan-core.inputs.data-mesher.inputs.treefmt-nix.follows = "treefmt-nix"`
/// that contradicts the parent's chosen target.
#[rstest]
fn test_follow_respects_ancestor() {
    let mut settings = insta::Settings::clone_current();
    path_redactions(&mut settings);
    stderr_path_filters(&mut settings);
    settings.bind(|| {
        assert_cmd_snapshot!(
            cli()
                .arg("--flake")
                .arg(fixture_path("follow_respects_ancestor"))
                .arg("--lock-file")
                .arg(fixture_lock_path("follow_respects_ancestor"))
                .arg("--diff")
                .arg("follow")
                .arg("--transitive")
                .arg("--depth")
                .arg("6")
        );
    });
}

/// `inputs.X.follows = ""` is the user explicitly nulling a nested input.
/// `--transitive --depth 6` against a flake with a top-level `treefmt-nix`
/// must not propose to retarget the nulled follows: doing so would erase
/// the user's deliberate decision.
#[rstest]
fn test_follow_respects_nulled() {
    let mut settings = insta::Settings::clone_current();
    path_redactions(&mut settings);
    stderr_path_filters(&mut settings);
    settings.bind(|| {
        assert_cmd_snapshot!(
            cli()
                .arg("--flake")
                .arg(fixture_path("follow_respects_nulled"))
                .arg("--lock-file")
                .arg(fixture_lock_path("follow_respects_nulled"))
                .arg("--diff")
                .arg("follow")
                .arg("--transitive")
                .arg("--depth")
                .arg("6")
        );
    });
}

/// Nulled twin of [`test_follow_empty_target_follows`]: `follows = ""`
/// whose source path is absent from the lockfile must be removed.
#[rstest]
fn test_follow_nulled_stale_source_removed() {
    let mut settings = insta::Settings::clone_current();
    path_redactions(&mut settings);
    stderr_path_filters(&mut settings);
    settings.bind(|| {
        assert_cmd_snapshot!(
            cli()
                .arg("--flake")
                .arg(fixture_path("nulled_stale_source_removed"))
                .arg("--lock-file")
                .arg(fixture_lock_path("nulled_stale_source_removed"))
                .arg("--diff")
                .arg("follow")
                .arg("--transitive")
                .arg("--depth")
                .arg("6")
        );
    });
}

/// Depth-3 sibling of [`test_follow_nulled_stale_source_removed`].
#[rstest]
fn test_follow_nulled_stale_depth3() {
    let mut settings = insta::Settings::clone_current();
    path_redactions(&mut settings);
    stderr_path_filters(&mut settings);
    settings.bind(|| {
        assert_cmd_snapshot!(
            cli()
                .arg("--flake")
                .arg(fixture_path("nulled_stale_depth3"))
                .arg("--lock-file")
                .arg(fixture_lock_path("nulled_stale_depth3"))
                .arg("--diff")
                .arg("follow")
                .arg("--transitive")
                .arg("--depth")
                .arg("6")
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

/// `follow` on a flake with no `inputs = { ... };` block exits 0 with
/// a no-op message instead of erroring with `No inputs found in the
/// flake`. The lock is borrowed from `first_nested_node` so the
/// fixture's empty-inputs case is what trips the no-op, not an empty
/// nested-inputs lockfile (both produce the same exit but only the
/// former regresses).
#[test]
fn test_follow_no_inputs() {
    let mut settings = insta::Settings::clone_current();
    path_redactions(&mut settings);
    stderr_path_filters(&mut settings);
    settings.bind(|| {
        assert_cmd_snapshot!(
            cli()
                .arg("--flake")
                .arg(fixture_path("follow_no_inputs"))
                .arg("--lock-file")
                .arg(fixture_lock_path("first_nested_node"))
                .arg("--no-lock")
                .arg("--diff")
                .arg("follow")
        );
    });
}

/// Batch mode (`flake-edit follow PATHS...`) on a mix of inputful and
/// inputless flakes exits 0, emits the inputful flake's diff, and
/// produces no `Error processing ...` line for the inputless one.
#[test]
fn test_follow_paths_batch_with_inputless() {
    let tmpdir = tempfile::tempdir().expect("Failed to create tmpdir");
    let root = tmpdir.path();

    let inputful_dir = root.join("inputful");
    let inputless_dir = root.join("inputless");
    fs::create_dir(&inputful_dir).expect("create inputful/");
    fs::create_dir(&inputless_dir).expect("create inputless/");

    copy_fixture_to_dir("centerpiece", &inputful_dir);

    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    fs::copy(
        format!("{manifest_dir}/tests/fixtures/follow_no_inputs.flake.nix"),
        inputless_dir.join("flake.nix"),
    )
    .expect("copy inputless flake.nix");
    // Borrow any lock with nested_inputs so loading it succeeds;
    // missing-lock handling is governed elsewhere.
    fs::copy(
        format!("{manifest_dir}/tests/fixtures/first_nested_node.flake.lock"),
        inputless_dir.join("flake.lock"),
    )
    .expect("copy inputless flake.lock");

    let output = Command::new(get_cargo_bin("flake-edit"))
        .env("NO_COLOR", "1")
        .arg("--diff")
        .arg("follow")
        .arg(inputful_dir.join("flake.nix"))
        .arg(inputless_dir.join("flake.nix"))
        .output()
        .expect("Failed to run flake-edit");

    assert!(
        output.status.success(),
        "batch follow failed (status {:?}): stdout={}\nstderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("Error processing"),
        "inputless flake produced an error in batch mode: stderr={stderr}",
    );
    assert!(
        !stderr.contains("No inputs found"),
        "inputless flake reported NoInputs: stderr={stderr}",
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("home-manager") || stdout.contains("treefmt"),
        "inputful flake's diff did not appear in stdout: stdout={stdout}",
    );
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

/// Test that `pin` respects `--lock-file` flag instead of reading `./flake.lock` from CWD.
#[rstest]
#[case("root")]
fn test_pin_with_lock_file(#[case] fixture: &str) {
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
                .arg("pin")
                .arg("nixpkgs")
        );
    });
}
