mod common;

use common::{Info, load_flake};
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
fn test_flake_edit_list(#[case] fixture: &str) {
    let content = load_flake(fixture);
    let mut flake_edit = FlakeEdit::from_text(&content).unwrap();
    let info = Info::empty();
    insta::with_settings!({
        sort_maps => true,
        info => &info,
        snapshot_suffix => fixture
    }, {
        insta::assert_yaml_snapshot!(flake_edit.list());
    });
}

#[rstest]
#[case("root", true, "github:mic92/vmsh")]
#[case("root", false, "github:a-kenji/not_a_flake")]
#[case("completely_flat_toplevel", true, "mic92/vmsh")]
#[case("completely_flat_toplevel", false, "github:a-kenji/not_a_flake")]
#[case("flat_nested_flat", true, "mic92/vmsh")]
#[case("flat_nested_flat", false, "github:a-kenji/not_a_flake")]
fn test_add_input(#[case] fixture: &str, #[case] is_flake: bool, #[case] uri: &str) {
    let content = load_flake(fixture);
    let mut flake_edit = FlakeEdit::from_text(&content).unwrap();
    let id = if is_flake { "vmsh" } else { "not_a_flake" };
    let change = Change::Add {
        id: Some(id.to_owned()),
        uri: Some(uri.to_owned()),
        flake: is_flake,
    };
    let info = Info::with_change(change.clone());
    let result = flake_edit.apply_change(change).unwrap().unwrap();
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
        id: Some("home-manager".to_owned()),
        uri: Some("github:nix-community/home-manager/release-24.05".to_owned()),
        flake: true,
    };
    let info = Info::with_change(change.clone());
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(flake_edit.apply_change(change).unwrap().unwrap());
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
        id: Some(id.to_owned()),
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
        insta::assert_snapshot!("changes", flake_edit.apply_change(change).unwrap().unwrap());
    });
    insta::with_settings!({
        sort_maps => true,
        info => &info,
        snapshot_suffix => suffix
    }, {
        insta::assert_yaml_snapshot!("list", flake_edit.curr_list());
    });
}

#[rstest]
#[case("completely_flat_toplevel", "nixpkgs")]
#[case("completely_flat_toplevel", "crane")]
#[case("one_level_nesting_flat", "nixpkgs")]
#[case("one_level_nesting_flat", "rust-overlay")]
#[case("flat_nested_flat", "nixpkgs")]
#[case("flat_nested_flat", "poetry2nix")]
fn test_remove_input(#[case] fixture: &str, #[case] input_id: &str) {
    let content = load_flake(fixture);
    let mut flake_edit = FlakeEdit::from_text(&content).unwrap();
    let change = Change::Remove {
        ids: vec![input_id.to_owned().into()],
    };
    let info = Info::with_change(change.clone());
    let result = flake_edit.apply_change(change).unwrap().unwrap();
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
        ids: vec![input_id.to_owned().into()],
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
fn test_remove_nested_input(#[case] fixture: &str, #[case] input_id: &str) {
    let content = load_flake(fixture);
    let mut flake_edit = FlakeEdit::from_text(&content).unwrap();
    let change = Change::Remove {
        ids: vec![input_id.to_owned().into()],
    };
    let info = Info::with_change(change.clone());
    let result = flake_edit.apply_change(change).unwrap().unwrap();
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
        ids: vec![input_id.to_owned().into()],
    };
    let info = Info::with_change(change.clone());
    let result = flake_edit.apply_change(change).unwrap().unwrap();
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
        ids: vec![input_id.to_owned().into()],
    };
    let info = Info::with_change(change.clone());
    insta::with_settings!({
        sort_maps => true,
        info => &info,
        snapshot_suffix => input_id
    }, {
        insta::assert_snapshot!("changes", flake_edit.apply_change(change).unwrap().unwrap());
    });
    insta::with_settings!({
        sort_maps => true,
        info => &info,
        snapshot_suffix => input_id
    }, {
        insta::assert_yaml_snapshot!("list", flake_edit.list());
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
        id: Some(input_id.to_owned()),
        uri: Some(new_url.to_owned()),
        ref_or_rev: None,
    };
    let info = Info::with_change(change.clone());
    let result = flake_edit.apply_change(change).unwrap().unwrap();
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
#[case("root", "nonexistent")]
#[case("completely_flat_toplevel", "nonexistent")]
fn test_change_nonexistent_input_error(#[case] fixture: &str, #[case] input_id: &str) {
    let content = load_flake(fixture);
    let mut flake_edit = FlakeEdit::from_text(&content).unwrap();
    let change = Change::Change {
        id: Some(input_id.to_owned()),
        uri: Some("github:foo/bar".to_owned()),
        ref_or_rev: None,
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
        ids: vec!["not-an-input-at-all".to_owned().into()],
    };
    flake_edit.apply_change(change).unwrap().unwrap();
}

#[rstest]
#[case("one_level_nesting_flat")]
#[case("first_nested_node")]
fn test_walker_inputs(#[case] fixture: &str) {
    let content = load_flake(fixture);
    let mut walker = Walker::new(&content);
    let _ = walker.walk(&Change::None);
    let info = Info::empty();
    insta::with_settings!({
        sort_maps => true,
        info => &info,
        snapshot_suffix => fixture
    }, {
        insta::assert_yaml_snapshot!(walker.inputs);
    });
}
