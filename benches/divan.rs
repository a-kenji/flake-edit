use divan::Bencher;
use flake_edit::app::commands::follow::auto;
use flake_edit::cache::Cache;
use flake_edit::change::Change;
use flake_edit::config::FollowConfig;
use flake_edit::walk::Walker;

fn main() {
    divan::main();
}

const INPUTS: &str = r#"{
      description = "Manage your flake inputs comfortably.";

      inputs = {
        flake-utils.url = "github:numtide/flake-utils";
        flake-utils.flake = false;
        rust-overlay = {
          url = "github:oxalica/rust-overlay";
          inputs.flake-utils.follows = "flake-utils";
        };
      };
      }
    "#;

const HYPERCONFIG_FLAKE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/hyperconfig.flake.nix"
));
const HYPERCONFIG_LOCK: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/hyperconfig.flake.lock"
));

#[divan::bench]
fn collect_inputs() {
    let mut walker = Walker::new(INPUTS);
    walker
        .walk(&Change::None)
        .expect("bug: bench fixture must walk cleanly");
}

#[divan::bench]
fn add_input() {
    let mut walker = Walker::new(INPUTS);
    let change = Change::Add {
        id: Some(flake_edit::change::ChangeId::parse("nixpkgs").unwrap()),
        uri: Some("github/nixos/nixpkgs".to_owned()),
        flake: true,
    };
    walker
        .walk(&change)
        .expect("bug: bench fixture must walk cleanly");
}

#[divan::bench]
fn remove_input() {
    let mut walker = Walker::new(INPUTS);
    let change = Change::Remove {
        ids: vec![flake_edit::change::ChangeId::parse("nixpkgs").unwrap()],
    };
    walker
        .walk(&change)
        .expect("bug: bench fixture must walk cleanly");
}

#[divan::bench]
fn follow_large_fixture(bencher: Bencher) {
    // Construct the planner config once so only the planner is timed.
    let follow_config = FollowConfig {
        transitive_min: 2,
        max_depth: Some(8),
        ..FollowConfig::default()
    };
    bencher.bench_local(|| {
        let output = auto::run_in_memory(HYPERCONFIG_FLAKE, HYPERCONFIG_LOCK, &follow_config)
            .expect("follow benchmark fixture succeeds");
        assert!(
            output.is_some(),
            "hyperconfig fixture should still produce edits"
        );
        output
    });
}

fn large_completion_cache() -> tempfile::TempDir {
    let mut cache = Cache::default();
    for i in 0..1000 {
        cache.add_entry(
            format!("input-{i}"),
            format!("github:example-owner-{i}/example-repository-{i}/nixos-unstable"),
        );
    }
    let dir = tempfile::tempdir().expect("bench tempdir");
    let json = serde_json::to_vec(&cache).expect("serialize bench cache");
    std::fs::write(dir.path().join("flake_edit.json"), json).expect("write bench cache");
    dir
}

#[divan::bench]
fn load_completion_cache(bencher: Bencher) {
    let dir = large_completion_cache();
    let path = dir.path().join("flake_edit.json");
    bencher.bench(|| Cache::try_from_path(&path).expect("bench cache loads"));
}
