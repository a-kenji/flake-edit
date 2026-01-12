mod common;

use common::{Info, load_flake};
use flake_edit::change::Change;
use flake_edit::walk::Walker;
use rstest::rstest;

#[rstest]
#[case("root")]
#[case("root_alt")]
#[case("toplevel_nesting")]
#[case("completely_flat_toplevel")]
#[case("completely_flat_toplevel_alt")]
#[case("one_level_nesting_flat")]
#[case("flat_nested_flat")]
fn test_walker_list_inputs(#[case] fixture: &str) {
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

#[rstest]
#[case("root", true)]
#[case("root", false)]
#[case("root_alt", true)]
#[case("root_alt", false)]
fn test_walker_add_input(#[case] fixture: &str, #[case] is_flake: bool) {
    let content = load_flake(fixture);
    let mut walker = Walker::new(&content);
    let (id, uri) = if is_flake {
        ("vmsh", "github:mic92/vmsh")
    } else {
        ("not_a_flake", "github:a-kenji/not_a_flake")
    };
    let change = Change::Add {
        id: Some(id.to_owned()),
        uri: Some(uri.to_owned()),
        flake: is_flake,
    };
    let info = Info::with_change(change.clone());
    let result = walker.walk(&change).unwrap().unwrap();
    let suffix = format!("{}_flake_{}", fixture, is_flake);
    insta::with_settings!({
        sort_maps => true,
        info => &info,
        snapshot_suffix => suffix
    }, {
        insta::assert_snapshot!(result.to_string());
    });
}

#[rstest]
#[case("flat_nested_flat", "poetry2nix")]
#[case("flat_nested_flat", "poetry2nix.nixpkgs")]
fn test_walker_remove_input(#[case] fixture: &str, #[case] input_id: &str) {
    let content = load_flake(fixture);
    let mut walker = Walker::new(&content);
    let change = Change::Remove {
        id: input_id.to_owned().into(),
    };
    let info = Info::with_change(change.clone());
    let result = walker.walk(&change).unwrap().unwrap();
    let suffix = format!("{}_{}", fixture, input_id.replace('.', "_"));
    insta::with_settings!({
        sort_maps => true,
        info => &info,
        snapshot_suffix => suffix
    }, {
        insta::assert_snapshot!(result.to_string());
    });
}
