use flake_edit::edit::FlakeEdit;
use flake_edit::update::Updater;
use ropey::Rope;

fn flake_with_pins() -> String {
    r#"{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils?rev=abcdef";
  };

  outputs = { self, nixpkgs, flake-utils }: { };
}
"#
    .to_string()
}

#[test]
fn unpin_removes_ref_param() {
    let flake = flake_with_pins();
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let inputs = flake_edit.list().clone();
    let mut updater = Updater::new(flake.into(), inputs);

    updater.unpin_input("nixpkgs");

    insta::assert_snapshot!(updater.get_changes());
}

#[test]
fn unpin_removes_rev_param() {
    let flake = flake_with_pins();
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let inputs = flake_edit.list().clone();
    let mut updater = Updater::new(flake.into(), inputs);

    updater.unpin_input("flake-utils");

    insta::assert_snapshot!(updater.get_changes());
}

#[test]
fn unpin_follows_before_url() {
    let flake = r#"{
  inputs = {
    myInput = {
      inputs.nixpkgs.follows = "nixpkgs";
      url = "github:foo/bar?ref=some-branch";
    };
  };

  outputs = { self, myInput }: { };
}
"#
    .to_string();
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let inputs = flake_edit.list().clone();
    let mut updater = Updater::new(Rope::from_str(&flake), inputs);

    updater.unpin_input("myInput");

    insta::assert_snapshot!(updater.get_changes());
}

#[test]
fn unpin_expanded_format_no_crash() {
    let flake = r#"{
  inputs = {
    myInput = {
      type = "github";
      owner = "NixOS";
      repo = "nixpkgs";
      ref = "nixos-25.11";
    };
    pinned.url = "github:foo/bar?ref=v1.0";
  };

  outputs = { self, myInput, pinned }: { };
}
"#
    .to_string();
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let inputs = flake_edit.list().clone();
    let mut updater = Updater::new(Rope::from_str(&flake), inputs);

    // Should not crash even though myInput has no url
    updater.unpin_input("pinned");

    insta::assert_snapshot!(updater.get_changes());
}

#[test]
fn pin_follows_before_url() {
    let flake = r#"{
  inputs = {
    myInput = {
      inputs.nixpkgs.follows = "nixpkgs";
      url = "github:foo/bar";
    };
  };

  outputs = { self, myInput }: { };
}
"#
    .to_string();
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let inputs = flake_edit.list().clone();
    let mut updater = Updater::new(Rope::from_str(&flake), inputs);

    updater.pin_input_to_ref("myInput", "abc123");

    insta::assert_snapshot!(updater.get_changes());
}
