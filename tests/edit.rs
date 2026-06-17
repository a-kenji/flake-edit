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
fn list_reports_inputs_in_let_wrapped_flake() {
    // A flake whose root expression is `let ... in { ... }` keeps its
    // inputs in the `in` body. The walker must descend past the let
    // bindings to find them, otherwise it reports zero inputs.
    let content = load_flake("let_wrapped");
    let mut flake_edit = FlakeEdit::from_text(&content).unwrap();
    let inputs = flake_edit.list();
    assert!(
        inputs.contains_key("nixpkgs"),
        "let-wrapped flake should report its `nixpkgs` input, got: {:?}",
        inputs.keys().collect::<Vec<_>>()
    );
}

#[test]
fn add_reaches_body_of_let_wrapped_flake() {
    // `add` on a let-wrapped flake must reach the body attrset rather
    // than erroring on the `let` bindings at the top level.
    let content = load_flake("let_wrapped");
    let mut flake_edit = FlakeEdit::from_text(&content).unwrap();
    let change = Change::Add {
        id: Some(flake_edit::change::ChangeId::parse("vmsh").unwrap()),
        uri: Some("github:mic92/vmsh".to_owned()),
        flake: true,
    };
    let text = flake_edit
        .apply_change(change)
        .expect("add on a let-wrapped flake must not error at the top level")
        .text
        .expect("add must produce changed text");
    assert!(
        text.contains("vmsh"),
        "added input should appear in the body, got:\n{text}"
    );
    assert!(
        text.contains("system = \"x86_64-linux\""),
        "the `let` binding must be preserved, got:\n{text}"
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

/// Apply a `Change::Toggle` to `content` and return the resulting text.
fn apply_toggle(content: &str, id: &str, uri: &str, previous: &str) -> String {
    let mut flake_edit = FlakeEdit::from_text(content).unwrap();
    let change = Change::Toggle {
        id: flake_edit::change::ChangeId::parse(id).unwrap(),
        uri: uri.to_owned(),
        previous: previous.to_owned(),
    };
    flake_edit
        .apply_change(change)
        .expect("apply Change::Toggle must succeed")
        .text
        .expect("toggle must produce changed text")
}

#[test]
fn toggle_states_detects_alternate_above_flat_url() {
    let content = load_flake("toggle_flat");
    let mut flake_edit = FlakeEdit::from_text(&content).unwrap();
    let states = flake_edit.toggle_states().unwrap();
    let state = &states["rust-overlay"];
    assert_eq!(state.active, "github:oxalica/rust-overlay");
    assert_eq!(state.alternates, vec!["github:a-kenji/rust-overlay"]);
    assert!(states["nixpkgs"].alternates.is_empty());
}

#[test]
fn toggle_states_orders_alternates_above_then_below() {
    let content = r#"{
  inputs = {
    # crane.url = "github:a-kenji/crane";
    crane.url = "github:ipetkov/crane";
    # crane.url = "path:../crane";
    nixpkgs.url = "github:nixos/nixpkgs";
  };
  outputs = _: { };
}
"#;
    let mut flake_edit = FlakeEdit::from_text(content).unwrap();
    let states = flake_edit.toggle_states().unwrap();
    assert_eq!(
        states["crane"].alternates,
        vec!["github:a-kenji/crane", "path:../crane"],
    );
}

#[test]
fn toggle_states_ignores_prose_blank_line_and_wrong_attr_comments() {
    let content = r#"{
  # SPDX-License-Identifier: MIT

  inputs = {
    # this needs to be rolling so we're testing what most devs are using
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";

    # crane.url = "github:a-kenji/crane";

    crane.url = "github:ipetkov/crane";
    # nixpkgs.url = "github:someone/nixpkgs";
    fenix.url = "github:nix-community/fenix";
  };
  outputs = _: { };
}
"#;
    let mut flake_edit = FlakeEdit::from_text(content).unwrap();
    let states = flake_edit.toggle_states().unwrap();
    // The prose comment does not parse as a binding.
    assert!(
        states["nixpkgs"].alternates.is_empty(),
        "prose comment must not count"
    );
    // The crane alternate is separated from the binding by a blank line.
    assert!(
        states["crane"].alternates.is_empty(),
        "blank line breaks adjacency"
    );
    // The nixpkgs-flavoured comment above fenix binds a different attribute.
    assert!(
        states["fenix"].alternates.is_empty(),
        "wrong attribute must not count"
    );
}

#[test]
fn toggle_states_requires_string_literal_value() {
    let content = r#"{
  inputs = {
    # crane.url = true;
    crane.url = "github:ipetkov/crane";
  };
  outputs = _: { };
}
"#;
    let mut flake_edit = FlakeEdit::from_text(content).unwrap();
    let states = flake_edit.toggle_states().unwrap();
    assert!(states["crane"].alternates.is_empty());
}

#[test]
fn toggle_states_detects_block_form_and_quoted_ids() {
    let content = r#"{
  inputs = {
    rust-overlay = {
      # url = "github:a-kenji/rust-overlay";
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    # "hls-1.10".url = "github:haskell/haskell-language-server/1.10.0.0";
    "hls-1.10".url = "github:haskell/haskell-language-server";
    nixpkgs.url = "github:nixos/nixpkgs";
  };
  outputs = _: { };
}
"#;
    let mut flake_edit = FlakeEdit::from_text(content).unwrap();
    let states = flake_edit.toggle_states().unwrap();
    assert_eq!(
        states["rust-overlay"].alternates,
        vec!["github:a-kenji/rust-overlay"],
    );
    assert_eq!(
        states["hls-1.10"].alternates,
        vec!["github:haskell/haskell-language-server/1.10.0.0"],
    );
}

#[test]
fn toggle_states_detects_toplevel_flat_alternate() {
    let content = load_flake("toggle_toplevel_flat");
    let mut flake_edit = FlakeEdit::from_text(&content).unwrap();
    let states = flake_edit.toggle_states().unwrap();
    assert_eq!(states["crane"].alternates, vec!["github:a-kenji/crane"]);
    // A bare `# crane.url = ...` at top level would bind `crane.url`, not
    // `inputs.crane.url`. Only the `inputs.`-prefixed spelling counts there.
    assert!(states["nixpkgs"].alternates.is_empty());
}

#[test]
fn toggle_flip_moves_only_the_comment_marker() {
    let content = load_flake("toggle_flat");
    let result = apply_toggle(
        &content,
        "rust-overlay",
        "github:a-kenji/rust-overlay",
        "github:oxalica/rust-overlay",
    );
    let expected = r#"{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:a-kenji/rust-overlay";
    # rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
  };
  outputs = args: import ./nix args;
}
"#;
    assert_eq!(result, expected);
}

#[test]
fn toggle_twice_is_byte_identical() {
    let content = load_flake("toggle_flat");
    let once = apply_toggle(
        &content,
        "rust-overlay",
        "github:a-kenji/rust-overlay",
        "github:oxalica/rust-overlay",
    );
    let twice = apply_toggle(
        &once,
        "rust-overlay",
        "github:oxalica/rust-overlay",
        "github:a-kenji/rust-overlay",
    );
    assert_eq!(
        twice, content,
        "double toggle must restore the file byte for byte"
    );
}

#[test]
fn toggle_synthesizes_new_alternate_below_active_in_block_form() {
    let content = load_flake("toggle_block");
    let result = apply_toggle(
        &content,
        "rust-overlay",
        "path:../rust-overlay",
        "github:oxalica/rust-overlay",
    );
    let expected = r#"{
  description = "A tool built on rust-overlay";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      # url = "github:oxalica/rust-overlay";
      url = "path:../rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, ... }: { };
}
"#;
    assert_eq!(result, expected);
}

#[test]
fn toggle_no_space_comment_normalizes_once_then_stays_stable() {
    let content = r#"{
  inputs = {
    #crane.url = "github:a-kenji/crane";
    crane.url = "github:ipetkov/crane";
  };
  outputs = _: { };
}
"#;
    let once = apply_toggle(
        content,
        "crane",
        "github:a-kenji/crane",
        "github:ipetkov/crane",
    );
    let twice = apply_toggle(
        &once,
        "crane",
        "github:ipetkov/crane",
        "github:a-kenji/crane",
    );
    let normalized = content.replace("#crane.url", "# crane.url");
    assert_eq!(
        twice, normalized,
        "the only permitted change is `#x` -> `# x`"
    );
    let third = apply_toggle(
        &twice,
        "crane",
        "github:a-kenji/crane",
        "github:ipetkov/crane",
    );
    assert_eq!(third, once);
    let fourth = apply_toggle(
        &third,
        "crane",
        "github:ipetkov/crane",
        "github:a-kenji/crane",
    );
    assert_eq!(
        fourth, twice,
        "round trips after normalization are byte-stable"
    );
}

#[test]
fn toggle_keeps_trailing_comment_on_its_line_in_both_directions() {
    let content = load_flake("toggle_toplevel_flat");
    let once = apply_toggle(
        &content,
        "crane",
        "github:a-kenji/crane",
        "github:ipetkov/crane",
    );
    let expected = r#"{
  inputs.crane.url = "github:a-kenji/crane";
  # inputs.crane.url = "github:ipetkov/crane"; # build tool
  inputs.nixpkgs.url = "github:nixos/nixpkgs";

  outputs = { self, ... }: { };
}
"#;
    assert_eq!(once, expected);
    let twice = apply_toggle(
        &once,
        "crane",
        "github:ipetkov/crane",
        "github:a-kenji/crane",
    );
    assert_eq!(twice, content);
}

/// Apply a `Change::ToggleRemove` to `content` and return the outcome text.
fn apply_toggle_remove(
    content: &str,
    id: &str,
    uri: &str,
    activate: Option<&str>,
) -> Option<String> {
    let mut flake_edit = FlakeEdit::from_text(content).unwrap();
    let change = Change::ToggleRemove {
        id: flake_edit::change::ChangeId::parse(id).unwrap(),
        uri: uri.to_owned(),
        activate: activate.map(str::to_owned),
    };
    flake_edit
        .apply_change(change)
        .expect("apply Change::ToggleRemove must succeed")
        .text
}

#[test]
fn toggle_remove_deletes_alternate_above_active() {
    let content = load_flake("toggle_flat");
    let result = apply_toggle_remove(
        &content,
        "rust-overlay",
        "github:a-kenji/rust-overlay",
        None,
    )
    .expect("removing a stored alternate must change the text");
    let expected = r#"{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
  };
  outputs = args: import ./nix args;
}
"#;
    assert_eq!(result, expected);
}

#[test]
fn toggle_remove_deletes_alternate_below_active() {
    let content = load_flake("toggle_flat_flipped");
    let result = apply_toggle_remove(
        &content,
        "rust-overlay",
        "github:oxalica/rust-overlay",
        None,
    )
    .expect("removing a stored alternate must change the text");
    let expected = r#"{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:a-kenji/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
  };
  outputs = args: import ./nix args;
}
"#;
    assert_eq!(result, expected);
}

#[test]
fn toggle_remove_active_activates_alternate_and_deletes_line() {
    let content = load_flake("toggle_flat");
    let result = apply_toggle_remove(
        &content,
        "rust-overlay",
        "github:oxalica/rust-overlay",
        Some("github:a-kenji/rust-overlay"),
    )
    .expect("removing the active url must change the text");
    let expected = r#"{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:a-kenji/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
  };
  outputs = args: import ./nix args;
}
"#;
    assert_eq!(result, expected);
}

#[test]
fn toggle_remove_active_takes_trailing_comment_along() {
    let content = load_flake("toggle_toplevel_flat");
    let result = apply_toggle_remove(
        &content,
        "crane",
        "github:ipetkov/crane",
        Some("github:a-kenji/crane"),
    )
    .expect("removing the active url must change the text");
    let expected = r#"{
  inputs.crane.url = "github:a-kenji/crane";
  inputs.nixpkgs.url = "github:nixos/nixpkgs";

  outputs = { self, ... }: { };
}
"#;
    assert_eq!(result, expected);
}

#[test]
fn toggle_synthesize_then_remove_restores_original() {
    let content = load_flake("toggle_block");
    let synthesized = apply_toggle(
        &content,
        "rust-overlay",
        "github:a-kenji/rust-overlay",
        "github:oxalica/rust-overlay",
    );
    let restored = apply_toggle_remove(
        &synthesized,
        "rust-overlay",
        "github:a-kenji/rust-overlay",
        Some("github:oxalica/rust-overlay"),
    )
    .expect("removing the synthesized variant must change the text");
    assert_eq!(
        restored, content,
        "removing through the active url must invert first-use synthesis byte for byte",
    );
}

#[test]
fn toggle_remove_unstored_uri_is_a_noop() {
    let content = load_flake("toggle_flat");
    let result = apply_toggle_remove(&content, "rust-overlay", "github:nobody/nothing", None);
    assert_eq!(result, None, "an unstored uri must be a no-op");
}

#[test]
fn toggle_remove_active_without_alternate_errors() {
    let content = load_flake("toggle_block");
    let mut flake_edit = FlakeEdit::from_text(&content).unwrap();
    let change = Change::ToggleRemove {
        id: flake_edit::change::ChangeId::parse("rust-overlay").unwrap(),
        uri: "github:oxalica/rust-overlay".to_owned(),
        activate: None,
    };
    let err = flake_edit.apply_change(change).expect_err("must refuse");
    assert!(
        err.to_string().contains("without an alternate"),
        "expected the remove-active error, got: {err}",
    );
}

#[test]
fn toggle_follows_only_input_has_no_url_to_toggle() {
    let content = load_flake("toggle_follows_only");
    let mut flake_edit = FlakeEdit::from_text(&content).unwrap();
    let states = flake_edit.toggle_states().unwrap();
    assert!(
        !states.contains_key("nixpkgs-lib"),
        "follows-only inputs have no toggle state",
    );
    let change = Change::Toggle {
        id: flake_edit::change::ChangeId::parse("nixpkgs-lib").unwrap(),
        uri: "github:nix-community/nixpkgs.lib".to_owned(),
        previous: String::new(),
    };
    let err = flake_edit.apply_change(change).expect_err("must refuse");
    assert!(
        err.to_string().contains("no url to toggle"),
        "expected the no-url error, got: {err}",
    );
}
