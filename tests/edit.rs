#![cfg(feature = "application")]

mod common;

use common::{Info, load_flake};
use flake_edit::app::commands::list::ListOutput;
use flake_edit::change::Change;
use flake_edit::edit::FlakeEdit;
use flake_edit::walk::Walker;
use rstest::rstest;

#[rstest]
#[case("root")]
#[case("root_alt")]
#[case("toplevel_nesting")]
#[case("completely_flat_toplevel")]
#[case("completely_flat_toplevel_alt")]
#[case("completely_flat_toplevel_not_a_flake")]
#[case("completely_flat_toplevel_not_a_flake_nested")]
#[case("one_level_nesting_flat_not_a_flake")]
#[case("merged_inputs")]
#[case("merged_inputs_flat")]
#[case("multi_hop_cycle")]
#[case("dot_ancestor_cycle")]
#[case("lockfile_only_cycle")]
#[case("stale_lockfile_only")]
#[case("follows_empty_target")]
#[case("nested_url_override")]
fn test_flake_edit_list(#[case] fixture: &str) {
    let content = load_flake(fixture);
    let mut flake_edit = FlakeEdit::from_text(&content).unwrap();
    let info = Info::empty();
    insta::with_settings!({
        sort_maps => true,
        info => &info,
        snapshot_suffix => fixture
    }, {
        insta::assert_yaml_snapshot!(ListOutput::from(flake_edit.list()));
    });
}

#[rstest]
#[case("outputs_at_no_space", true, "github:mic92/vmsh")]
#[case("outputs_at_space", true, "github:mic92/vmsh")]
#[case("root", true, "github:mic92/vmsh")]
#[case("root", false, "github:a-kenji/not_a_flake")]
#[case("completely_flat_toplevel", true, "mic92/vmsh")]
#[case("completely_flat_toplevel", false, "github:a-kenji/not_a_flake")]
#[case("flat_nested_flat", true, "mic92/vmsh")]
#[case("flat_nested_flat", false, "github:a-kenji/not_a_flake")]
#[case("leading_comma_outputs", true, "mic92/vmsh")]
#[case("merged_inputs", true, "github:mic92/vmsh")]
#[case("merged_inputs_flat", true, "github:mic92/vmsh")]
#[case("all_blanks", true, "github:mic92/vmsh")]
#[case("all_blanks", false, "github:a-kenji/not_a_flake")]
#[case("quoted_input_with_dots", true, "github:mic92/vmsh")]
#[case("outputs_no_space_add", false, "github:a-kenji/not_a_flake")]
#[case("outputs_at_no_space_multi", true, "github:mic92/vmsh")]
#[case("outputs_at_space_multi", true, "github:mic92/vmsh")]
#[case("outputs_at_leading_comma", true, "github:mic92/vmsh")]
#[case("outputs_at_space_args", true, "github:mic92/vmsh")]
#[case("multiline_no_trailing_comma_outputs", true, "github:mic92/vmsh")]
#[case("outputs_at_trailing_comma_multi", true, "github:mic92/vmsh")]
#[case("leading_comma_trailing_comma_outputs", true, "github:mic92/vmsh")]
#[case("outputs_at_leading_comma_trailing_comma", true, "github:mic92/vmsh")]
#[case("empty_inputs", true, "github:mic92/vmsh")]
#[case("empty_inputs", false, "github:a-kenji/not_a_flake")]
#[case("outputs_paren", true, "github:mic92/vmsh")]
#[case("outputs_no_space_add", true, "github:mic92/vmsh")]
fn test_add_input(#[case] fixture: &str, #[case] is_flake: bool, #[case] uri: &str) {
    let content = load_flake(fixture);
    let mut flake_edit = FlakeEdit::from_text(&content).unwrap();
    let id = if is_flake { "vmsh" } else { "not_a_flake" };
    let change = Change::Add {
        id: Some(flake_edit::change::ChangeId::parse(id).unwrap()),
        uri: Some(uri.to_owned()),
        flake: is_flake,
    };
    let info = Info::with_change(change.clone());
    let result = flake_edit.apply_change(change).unwrap().text.unwrap();
    let suffix = format!("{}_flake_{}", fixture, is_flake);
    insta::with_settings!({
        sort_maps => true,
        info => &info,
        snapshot_suffix => suffix
    }, {
        insta::assert_snapshot!(result);
    });
}

#[test]
fn test_add_with_ref_or_rev() {
    let content = load_flake("root");
    let mut flake_edit = FlakeEdit::from_text(&content).unwrap();
    let change = Change::Add {
        id: Some(flake_edit::change::ChangeId::parse("home-manager").unwrap()),
        uri: Some("github:nix-community/home-manager/release-24.05".to_owned()),
        flake: true,
    };
    let info = Info::with_change(change.clone());
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(flake_edit.apply_change(change).unwrap().text.unwrap());
    });
}

#[rstest]
#[case(true)]
#[case(false)]
fn test_first_nested_node_add_with_list(#[case] is_flake: bool) {
    let content = load_flake("first_nested_node");
    let mut flake_edit = FlakeEdit::from_text(&content).unwrap();
    let (id, uri) = if is_flake {
        ("vmsh", "mic92/vmsh")
    } else {
        ("not_a_flake", "github:a-kenji/not_a_flake")
    };
    let change = Change::Add {
        id: Some(flake_edit::change::ChangeId::parse(id).unwrap()),
        uri: Some(uri.to_owned()),
        flake: is_flake,
    };
    let info = Info::empty();
    let suffix = format!("flake_{}", is_flake);
    insta::with_settings!({
        sort_maps => true,
        info => &info,
        snapshot_suffix => suffix.clone()
    }, {
        insta::assert_snapshot!("changes", flake_edit.apply_change(change).unwrap().text.unwrap());
    });
    insta::with_settings!({
        sort_maps => true,
        info => &info,
        snapshot_suffix => suffix
    }, {
        insta::assert_yaml_snapshot!("list", ListOutput::from(flake_edit.curr_list()));
    });
}

#[rstest]
#[case("completely_flat_toplevel", "nixpkgs")]
#[case("completely_flat_toplevel", "crane")]
#[case("one_level_nesting_flat", "nixpkgs")]
#[case("one_level_nesting_flat", "rust-overlay")]
#[case("flat_nested_flat", "nixpkgs")]
#[case("flat_nested_flat", "poetry2nix")]
#[case("merged_inputs_flat", "extra")]
#[case("merged_inputs_flat", "nixpkgs")]
#[case("merged_inputs", "plugin-a")]
#[case("outputs_at_remove_only", "nixpkgs-lib")]
#[case("outputs_at_remove_first", "nixpkgs-lib")]
#[case("outputs_at_remove_multiline", "nixpkgs-lib")]
#[case("outputs_at_leading_comma", "fenix")]
#[case("leading_comma_outputs", "fenix")]
#[case("outputs_paren", "flake-parts")]
#[case("quoted_input_with_dots", "\"hls-1.10\"")]
#[case("outputs_no_space_remove", "flake-parts")]
#[case("follows_only_toplevel", "sizelint")]
#[case("follows_only_toplevel", "treefmt-nix")]
#[case("follows_only_nested", "sizelint")]
#[case("follows_only_nested", "treefmt-nix")]
fn test_remove_input(#[case] fixture: &str, #[case] input_id: &str) {
    let content = load_flake(fixture);
    let mut flake_edit = FlakeEdit::from_text(&content).unwrap();
    let change = Change::Remove {
        ids: vec![flake_edit::change::ChangeId::parse(input_id).unwrap()],
    };
    let info = Info::with_change(change.clone());
    let result = flake_edit.apply_change(change).unwrap().text.unwrap();
    let suffix = format!("{}_{}", fixture, input_id.replace('.', "_"));
    insta::with_settings!({
        sort_maps => true,
        info => &info,
        snapshot_suffix => suffix
    }, {
        insta::assert_snapshot!(result);
    });
}

#[rstest]
#[case("root", "nixpkgs")]
#[case("root", "crane")]
#[case("root_alt", "nixpkgs")]
#[case("root_alt", "crane")]
fn test_remove_input_walker(#[case] fixture: &str, #[case] input_id: &str) {
    let content = load_flake(fixture);
    let mut walker = Walker::new(&content);
    let change = Change::Remove {
        ids: vec![flake_edit::change::ChangeId::parse(input_id).unwrap()],
    };
    let info = Info::with_change(change.clone());
    let result = walker.walk(&change).unwrap().unwrap();
    let suffix = format!("{}_{}", fixture, input_id);
    insta::with_settings!({
        sort_maps => true,
        info => &info,
        snapshot_suffix => suffix
    }, {
        insta::assert_snapshot!(result.to_string());
    });
}

#[rstest]
#[case("root", "rust-overlay.flake-utils")]
#[case("completely_flat_toplevel", "crane.rust-overlay")]
#[case("one_level_nesting_flat", "rust-overlay.flake-utils")]
#[case("deeply_nested_inputs", "disko.nixpkgs")]
fn test_remove_nested_input(#[case] fixture: &str, #[case] input_id: &str) {
    let content = load_flake(fixture);
    let mut flake_edit = FlakeEdit::from_text(&content).unwrap();
    let change = Change::Remove {
        ids: vec![flake_edit::change::ChangeId::parse(input_id).unwrap()],
    };
    let info = Info::with_change(change.clone());
    let result = flake_edit.apply_change(change).unwrap().text.unwrap();
    let suffix = format!("{}_{}", fixture, input_id.replace('.', "_"));
    insta::with_settings!({
        sort_maps => true,
        info => &info,
        snapshot_suffix => suffix
    }, {
        insta::assert_snapshot!(result);
    });
}

#[rstest]
#[case("completely_flat_toplevel_not_a_flake", "not-a-flake")]
#[case("completely_flat_toplevel_not_a_flake_nested", "not-a-flake")]
#[case("one_level_nesting_flat_not_a_flake", "not-a-flake")]
fn test_remove_not_a_flake_input(#[case] fixture: &str, #[case] input_id: &str) {
    let content = load_flake(fixture);
    let mut flake_edit = FlakeEdit::from_text(&content).unwrap();
    let change = Change::Remove {
        ids: vec![flake_edit::change::ChangeId::parse(input_id).unwrap()],
    };
    let info = Info::with_change(change.clone());
    let result = flake_edit.apply_change(change).unwrap().text.unwrap();
    insta::with_settings!({
        sort_maps => true,
        info => &info,
        snapshot_suffix => fixture
    }, {
        insta::assert_snapshot!(result);
    });
}

#[rstest]
#[case("utils")]
#[case("naersk")]
fn test_first_nested_node_remove_with_list(#[case] input_id: &str) {
    let content = load_flake("first_nested_node");
    let mut flake_edit = FlakeEdit::from_text(&content).unwrap();
    let change = Change::Remove {
        ids: vec![flake_edit::change::ChangeId::parse(input_id).unwrap()],
    };
    let info = Info::with_change(change.clone());
    insta::with_settings!({
        sort_maps => true,
        info => &info,
        snapshot_suffix => input_id
    }, {
        insta::assert_snapshot!("changes", flake_edit.apply_change(change).unwrap().text.unwrap());
    });
    insta::with_settings!({
        sort_maps => true,
        info => &info,
        snapshot_suffix => input_id
    }, {
        insta::assert_yaml_snapshot!("list", ListOutput::from(flake_edit.list()));
    });
}

#[rstest]
#[case("root", "nixpkgs", "github:nixos/nixpkgs/nixos-24.05")]
#[case("root", "rust-overlay", "github:oxalica/rust-overlay/v1.0.0")]
#[case(
    "completely_flat_toplevel",
    "nixpkgs",
    "github:nixos/nixpkgs/nixos-24.05"
)]
#[case(
    "completely_flat_toplevel",
    "rust-overlay",
    "github:oxalica/rust-overlay/v1.0.0"
)]
#[case(
    "one_level_nesting_flat",
    "nixpkgs",
    "github:nixos/nixpkgs/nixos-24.05"
)]
#[case("flat_nested_flat", "nixpkgs", "github:nixos/nixpkgs/nixos-24.05")]
#[case("first_nested_node", "nixpkgs", "github:NixOS/nixpkgs/nixos-24.05")]
#[case("first_nested_node", "naersk", "github:nix-community/naersk/v1.0.0")]
fn test_change_url(#[case] fixture: &str, #[case] input_id: &str, #[case] new_url: &str) {
    let content = load_flake(fixture);
    let mut flake_edit = FlakeEdit::from_text(&content).unwrap();
    let change = Change::Change {
        id: Some(flake_edit::change::ChangeId::parse(input_id).unwrap()),
        uri: Some(new_url.to_owned()),
    };
    let info = Info::with_change(change.clone());
    let result = flake_edit.apply_change(change).unwrap().text.unwrap();
    let suffix = format!("{}_{}", fixture, input_id.replace('.', "_"));
    insta::with_settings!({
        sort_maps => true,
        info => &info,
        snapshot_suffix => suffix
    }, {
        insta::assert_snapshot!(result);
    });
}

/// A trailing `# comment` after the removed statement's semicolon belongs to
/// that line. Removing the input must drop the comment too, not move it onto
/// the surviving sibling above.
#[test]
fn remove_drops_trailing_comment_with_its_statement() {
    let content = load_flake("trailing_comment_on_removed_input");
    let mut flake_edit = FlakeEdit::from_text(&content).unwrap();
    let change = Change::Remove {
        ids: vec![flake_edit::change::ChangeId::parse("drop").unwrap()],
    };
    let result = flake_edit.apply_change(change).unwrap().text.unwrap();
    let expected = r#"{
  inputs = {
    keep.url = "github:owner/keep"; # keep me here
    after.url = "github:owner/after";
  };
  outputs = { self, ... }: { };
}
"#;
    assert_eq!(result, expected);
}

/// Inserting a follows after the last attribute of a block must land after a
/// trailing `# comment` on that attribute, leaving the comment on its original
/// statement rather than reattaching to the new follows line.
#[test]
fn add_follow_keeps_trailing_comment_on_sibling_attr() {
    let content = load_flake("trailing_comment_on_sibling_attr");
    let mut flake_edit = FlakeEdit::from_text(&content).unwrap();
    let change = Change::Follows {
        input: flake_edit::change::ChangeId::parse("dep.nixpkgs").unwrap(),
        target: flake_edit::follows::AttrPath::parse("nixpkgs").unwrap(),
    };
    let result = flake_edit.apply_change(change).unwrap().text.unwrap();
    let expected = r#"{
  inputs = {
    dep = {
      url = "github:owner/dep";
      flake = false; # just data, not a flake
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };
  outputs = { self, ... }: { };
}
"#;
    assert_eq!(result, expected);
}

#[rstest]
#[case("root", "nonexistent")]
#[case("completely_flat_toplevel", "nonexistent")]
fn test_change_nonexistent_input_error(#[case] fixture: &str, #[case] input_id: &str) {
    let content = load_flake(fixture);
    let mut flake_edit = FlakeEdit::from_text(&content).unwrap();
    let change = Change::Change {
        id: Some(flake_edit::change::ChangeId::parse(input_id).unwrap()),
        uri: Some("github:foo/bar".to_owned()),
    };
    let result = flake_edit.apply_change(change);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}

#[test]
#[should_panic]
fn test_remove_nonexistent_input_panics() {
    let content = load_flake("completely_flat_toplevel_not_a_flake");
    let mut flake_edit = FlakeEdit::from_text(&content).unwrap();
    let change = Change::Remove {
        ids: vec![flake_edit::change::ChangeId::parse("not-an-input-at-all").unwrap()],
    };
    flake_edit.apply_change(change).unwrap().text.unwrap();
}

#[test]
fn follows_fills_multiline_empty_inputs_block() {
    let content = load_flake("wrapper_with_empty_inputs_block");
    let mut flake_edit = FlakeEdit::from_text(&content).unwrap();
    let change = Change::Follows {
        input: flake_edit::change::ChangeId::parse("stylix.systems").unwrap(),
        target: flake_edit::follows::AttrPath::parse("systems").unwrap(),
    };
    let text = flake_edit
        .apply_change(change)
        .expect("apply Change::Follows must succeed")
        .text
        .expect("walker must produce changed text");

    let expected_block = r#"    stylix = {
      url = "github:danth/stylix/release-25.11";
      inputs = {
        systems.follows = "systems";
      };
    };
"#;
    assert!(
        text.contains(expected_block),
        "stylix block should carry the new follow inside the previously empty multiline inputs block, got:\n{text}"
    );
}

#[test]
fn follows_fills_compact_empty_inputs_block() {
    let content = load_flake("wrapper_with_empty_inputs_block");
    let mut flake_edit = FlakeEdit::from_text(&content).unwrap();
    let change = Change::Follows {
        input: flake_edit::change::ChangeId::parse("disko.systems").unwrap(),
        target: flake_edit::follows::AttrPath::parse("systems").unwrap(),
    };
    let text = flake_edit
        .apply_change(change)
        .expect("apply Change::Follows must succeed")
        .text
        .expect("walker must produce changed text");

    let expected_block = r#"    disko = {
      url = "github:nix-community/disko";
      inputs = {
        systems.follows = "systems";
      };
    };
"#;
    assert!(
        text.contains(expected_block),
        "disko block should carry the new follow inside the previously single-line empty inputs block, got:\n{text}"
    );
}

#[test]
fn follows_fills_empty_block_when_dotted_sibling_present() {
    // The empty block is filled and the pre-existing dotted sibling is
    // left in place. Promoting it would be a structural rewrite outside
    // the scope of an insertion-point decision.
    let content = load_flake("wrapper_with_empty_inputs_block");
    let mut flake_edit = FlakeEdit::from_text(&content).unwrap();
    let change = Change::Follows {
        input: flake_edit::change::ChangeId::parse("mixed.systems").unwrap(),
        target: flake_edit::follows::AttrPath::parse("systems").unwrap(),
    };
    let text = flake_edit
        .apply_change(change)
        .expect("apply Change::Follows must succeed")
        .text
        .expect("walker must produce changed text");

    let expected_block = r#"    mixed = {
      url = "github:owner/mixed";
      inputs = {
        systems.follows = "systems";
      };
      inputs.flake-parts.follows = "flake-parts";
    };
"#;
    assert!(
        text.contains(expected_block),
        "mixed block should fill the empty inputs block while leaving the dotted sibling in place, got:\n{text}"
    );
}

#[test]
fn follows_fill_empty_block_is_idempotent() {
    let content = load_flake("wrapper_with_empty_inputs_block");
    let mut first = FlakeEdit::from_text(&content).unwrap();
    let change = Change::Follows {
        input: flake_edit::change::ChangeId::parse("stylix.systems").unwrap(),
        target: flake_edit::follows::AttrPath::parse("systems").unwrap(),
    };
    let after_first = first
        .apply_change(change.clone())
        .expect("apply Change::Follows must succeed")
        .text
        .expect("walker must produce changed text");

    let mut second = FlakeEdit::from_text(&after_first).unwrap();
    let outcome = second
        .apply_change(change)
        .expect("apply Change::Follows must succeed");
    let after_second = outcome.text.unwrap_or_else(|| after_first.clone());

    assert_eq!(
        after_first, after_second,
        "second apply of the same follow on a filled block must be a no-op"
    );
}

#[test]
fn follows_merges_into_existing_inputs_block() {
    let content = load_flake("inputs_block_with_follows");
    let mut flake_edit = FlakeEdit::from_text(&content).unwrap();
    let change = Change::Follows {
        input: flake_edit::change::ChangeId::parse("stylix.systems").unwrap(),
        target: flake_edit::follows::AttrPath::parse("systems").unwrap(),
    };
    let text = flake_edit
        .apply_change(change)
        .expect("apply Change::Follows must succeed")
        .text
        .expect("walker must produce changed text");

    let expected_block = r#"    stylix = {
      url = "github:danth/stylix/release-25.11";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        flake-parts.follows = "flake-parts";
        systems.follows = "systems";
      };
    };
"#;
    assert!(
        text.contains(expected_block),
        "stylix block should carry the merged `systems.follows` inside the existing inputs block, got:\n{text}"
    );
}
