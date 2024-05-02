use flake_edit::change::Change;
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

#[divan::bench]
fn collect_inputs() {
    let mut walker = Walker::new(INPUTS);
    walker.walk(&Change::None);
    // a simple sanity check
    assert!(!walker.inputs.is_empty())
}

#[divan::bench]
fn add_input() {
    let mut walker = Walker::new(INPUTS);
    let change = Change::Add {
        id: Some("nixpkgs".to_owned()),
        uri: Some("github/nixos/nixpkgs".to_owned()),
        flake: true,
    };
    walker.walk(&change);
    // a simple sanity check
    assert!(!walker.inputs.is_empty())
}

#[divan::bench]
fn remove_input() {
    let mut walker = Walker::new(INPUTS);
    let change = Change::Remove {
        id: "nixpkgs".to_owned().into(),
    };
    walker.walk(&change);
    // a simple sanity check
    assert!(!walker.inputs.is_empty())
}
