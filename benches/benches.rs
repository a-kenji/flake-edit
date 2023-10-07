use criterion::{criterion_group, criterion_main, Criterion};
use flake_add::walk::Walker;

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

fn collect_inputs() {
    let mut walker = Walker::new(INPUTS).unwrap();
    walker.walk_toplevel();
    // a simple sanity check
    assert!(!walker.inputs.is_empty())
}

fn add_input() {
    let mut walker = Walker::new(INPUTS).unwrap();
    let change = flake_add::Change::Add {
        id: Some("nixpkgs".to_owned()),
        uri: Some("github/nixos/nixpkgs".to_owned()),
    };
    walker.changes.push(change);
    walker.walk_toplevel();
    // a simple sanity check
    assert!(!walker.inputs.is_empty())
}

fn remove_input() {
    let mut walker = Walker::new(INPUTS).unwrap();
    let change = flake_add::Change::Remove {
        id: "nixpkgs".to_owned(),
    };
    walker.changes.push(change);
    walker.walk_toplevel();
    // a simple sanity check
    assert!(!walker.inputs.is_empty())
}

fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function("collect_inputs", |b| b.iter(collect_inputs));
    c.bench_function("add_input", |b| b.iter(add_input));
    c.bench_function("remove_input", |b| b.iter(remove_input));
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
