use criterion::{Criterion, criterion_group, criterion_main};
use flake_edit::app::follow::auto;
use flake_edit::change::Change;
use flake_edit::config::FollowConfig;
use flake_edit::walk::Walker;

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

fn collect_inputs() {
    let mut walker = Walker::new(INPUTS);
    let _ = walker.walk(&Change::None);
    // a simple sanity check
    assert!(!walker.inputs.is_empty())
}

fn add_input() {
    let mut walker = Walker::new(INPUTS);
    let change = Change::Add {
        id: Some("nixpkgs".to_owned()),
        uri: Some("github/nixos/nixpkgs".to_owned()),
        flake: false,
    };
    let _ = walker.walk(&change);
    // a simple sanity check
    assert!(!walker.inputs.is_empty())
}

fn remove_input() {
    let mut walker = Walker::new(INPUTS);
    let change = Change::Remove {
        ids: vec![flake_edit::change::ChangeId::parse("nixpkgs").unwrap()],
    };
    let _ = walker.walk(&change);
    // a simple sanity check
    assert!(!walker.inputs.is_empty())
}

fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function("collect_inputs", |b| b.iter(collect_inputs));
    c.bench_function("add_input", |b| b.iter(add_input));
    c.bench_function("remove_input", |b| b.iter(remove_input));
    c.bench_function("follow_large_fixture", |b| {
        // Construct the planner config once so only the planner is timed.
        let follow_config = FollowConfig {
            transitive_min: 2,
            max_depth: 8,
            ..FollowConfig::default()
        };
        b.iter(|| {
            let output = auto::run_in_memory(HYPERCONFIG_FLAKE, HYPERCONFIG_LOCK, &follow_config)
                .expect("follow benchmark fixture succeeds");
            assert!(
                output.is_some(),
                "hyperconfig fixture should still produce edits"
            );
            output
        });
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
