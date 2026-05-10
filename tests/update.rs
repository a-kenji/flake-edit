use flake_edit::edit::FlakeEdit;
use flake_edit::forge::update::Updater;
use ropey::Rope;

fn flake_with_pins() -> String {
    r#"{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils?rev=0123456789abcdef0123456789abcdef01234567";
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

    updater
        .unpin_input("nixpkgs")
        .expect("unpin should succeed");

    insta::assert_snapshot!(updater.get_changes());
}

#[test]
fn unpin_removes_rev_param() {
    let flake = flake_with_pins();
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let inputs = flake_edit.list().clone();
    let mut updater = Updater::new(flake.into(), inputs);

    updater
        .unpin_input("flake-utils")
        .expect("unpin should succeed");

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

    updater
        .unpin_input("myInput")
        .expect("unpin should succeed");

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
    updater.unpin_input("pinned").expect("unpin should succeed");

    insta::assert_snapshot!(updater.get_changes());
}

#[test]
fn pin_quoted_input_by_bare_name() {
    let flake = r#"{
  inputs = {
    "nixpkgs-24.11".url = "github:nixos/nixpkgs/nixos-24.11";
  };

  outputs = { self, ... }: { };
}
"#
    .to_string();
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let inputs = flake_edit.list().clone();
    let mut updater = Updater::new(Rope::from_str(&flake), inputs);

    updater
        .pin_input_to_ref("nixpkgs-24.11", "abc123")
        .expect("bare name should match quoted input");

    insta::assert_snapshot!(updater.get_changes());
}

#[test]
fn unpin_quoted_input_by_bare_name() {
    let flake = r#"{
  inputs = {
    "nixpkgs-24.11".url = "github:nixos/nixpkgs?rev=50ab793786d9de88ee30ec4e4c24fb4236fc2674";
  };

  outputs = { self, ... }: { };
}
"#
    .to_string();
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let inputs = flake_edit.list().clone();
    let mut updater = Updater::new(Rope::from_str(&flake), inputs);

    updater
        .unpin_input("nixpkgs-24.11")
        .expect("bare name should match quoted input");

    insta::assert_snapshot!(updater.get_changes());
}

/// Regression: rnix TextRange yields *byte* offsets, but ropey slices by
/// *char* index. Any multibyte UTF-8 before the URL shifts the window and
/// pin/unpin would mangle the file.
#[test]
fn pin_with_multibyte_chars_before_url() {
    // "café →" contains a 2-byte and a 3-byte codepoint (3 extra bytes vs chars).
    let flake = r#"{
  # café → multibyte comment
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
  };

  outputs = { self, nixpkgs }: { };
}
"#
    .to_string();
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let inputs = flake_edit.list().clone();
    let mut updater = Updater::new(Rope::from_str(&flake), inputs);

    updater
        .pin_input_to_ref("nixpkgs", "50ab793786d9de88ee30ec4e4c24fb4236fc2674")
        .expect("pin should succeed");

    let result = updater.get_changes();
    // nix-uri may render the rev as a path segment or as `?rev=`; either is
    // valid flake-ref syntax. What matters is that the edit landed on the URL
    // and not three characters to the side of it.
    let pinned_path =
        r#"nixpkgs.url = "github:nixos/nixpkgs/50ab793786d9de88ee30ec4e4c24fb4236fc2674";"#;
    let pinned_query =
        r#"nixpkgs.url = "github:nixos/nixpkgs?rev=50ab793786d9de88ee30ec4e4c24fb4236fc2674";"#;
    assert!(
        result.contains(pinned_path) || result.contains(pinned_query),
        "URL was corrupted by byte/char offset mismatch:\n{result}"
    );
    assert!(
        result.contains("# café → multibyte comment"),
        "preceding text was corrupted:\n{result}"
    );
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

    updater
        .pin_input_to_ref("myInput", "abc123")
        .expect("pin should succeed");

    insta::assert_snapshot!(updater.get_changes());
}
