use flake_edit::change::Change;
use flake_edit::edit::FlakeEdit;
use flake_edit::walk::Walker;

fn load_fixtures(name: &str) -> (String, String) {
    let dir = env!("CARGO_MANIFEST_DIR");
    let flake_nix =
        std::fs::read_to_string(format!("{dir}/tests/fixtures/{name}.flake.nix")).unwrap();
    let flake_lock =
        std::fs::read_to_string(format!("{dir}/tests/fixtures/{name}.flake.lock")).unwrap();
    (flake_nix, flake_lock)
}

#[derive(serde::Serialize)]
struct Info {
    flake_nix: String,
    changes: Vec<Change>,
}

impl Info {
    fn new(flake_nix: String, changes: Vec<Change>) -> Self {
        Self { flake_nix, changes }
    }
}

#[test]
fn root_load() {
    let (_flake, _lock) = load_fixtures("root");
}

#[test]
fn root_edit_list() {
    let (flake, _lock) = load_fixtures("root");
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let info = Info::new("".into(), vec![]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_yaml_snapshot!(flake_edit.list());
    });
}
#[test]
fn root_add_toplevel_id_uri() {
    let (flake, _lock) = load_fixtures("root");
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let change = Change::Add {
        id: Some("vmsh".to_owned()),
        uri: Some("github:mic92/vmsh".to_owned()),
        flake: true,
    };
    let info = Info::new("".into(), vec![change.clone()]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(flake_edit.apply_change(change).unwrap().unwrap());
    });
}
#[test]
fn root_add_toplevel_id_uri_no_flake() {
    let (flake, _lock) = load_fixtures("root");
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let change = Change::Add {
        id: Some("not_a_flake".to_owned()),
        uri: Some("github:a-kenji/not_a_flake".to_owned()),
        flake: false,
    };
    let info = Info::new("".into(), vec![change.clone()]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(flake_edit.apply_change(change).unwrap().unwrap());
    });
}
#[test]
fn root_remove_toplevel_uri() {
    let (flake, _lock) = load_fixtures("root");
    let mut walker = Walker::new(&flake);
    let change = Change::Remove {
        id: "nixpkgs".to_owned().into(),
    };
    let info = Info::new("".into(), vec![change.clone()]);
    let change = walker.walk(&change).unwrap();
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(change.to_string());
    });
}
#[test]
fn root_remove_toplevel_input_multiple() {
    let (flake, _lock) = load_fixtures("root");
    let mut walker = Walker::new(&flake);
    let change = Change::Remove {
        id: "crane".to_owned().into(),
    };
    let info = Info::new("".into(), vec![change.clone()]);
    let change = walker.walk(&change).unwrap();
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(change.to_string());
    });
}
#[test]
fn root_remove_toplevel_input_single_nested() {
    let (flake, _lock) = load_fixtures("root");
    let mut walker = Walker::new(&flake);
    let change = Change::Remove {
        id: "rust-overlay.flake-utils".to_owned().into(),
    };
    let info = Info::new("".into(), vec![change.clone()]);
    let change = walker.walk(&change).unwrap();
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(change.to_string());
    });
}
#[test]
fn root_alt_list() {
    let (flake, _lock) = load_fixtures("root_alt");
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let info = Info::new("".into(), vec![]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_yaml_snapshot!(flake_edit.list());
    });
}
#[test]
fn root_alt_add_toplevel_id_uri() {
    let (flake, _lock) = load_fixtures("root_alt");
    let mut walker = Walker::new(&flake);
    let change = Change::Add {
        id: Some("vmsh".to_owned()),
        uri: Some("github:mic92/vmsh".to_owned()),
        flake: true,
    };
    let info = Info::new("".into(), vec![change.clone()]);
    let change = walker.walk(&change).unwrap();
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(change.to_string());
    });
}
#[test]
fn root_alt_add_toplevel_id_uri_no_flake() {
    let (flake, _lock) = load_fixtures("root_alt");
    let mut walker = Walker::new(&flake);
    let change = Change::Add {
        id: Some("not_a_flake".to_owned()),
        uri: Some("github:a-kenji/not_a_flake".to_owned()),
        flake: false,
    };
    let info = Info::new("".into(), vec![change.clone()]);
    let change = walker.walk(&change).unwrap();
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(change.to_string());
    });
}
#[test]
fn root_alt_remove_toplevel_uri() {
    let (flake, _lock) = load_fixtures("root_alt");
    let mut walker = Walker::new(&flake);
    let change = Change::Remove {
        id: "nixpkgs".to_string().into(),
    };
    let info = Info::new("".into(), vec![change.clone()]);
    let change = walker.walk(&change).unwrap();
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(change.to_string());
    });
}
#[test]
fn root_alt_remove_toplevel_input_multiple() {
    let (flake, _lock) = load_fixtures("root_alt");
    let mut walker = Walker::new(&flake);
    let change = Change::Remove {
        id: "crane".to_owned().into(),
    };
    let info = Info::new("".into(), vec![change.clone()]);
    let change = walker.walk(&change).unwrap();
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(change.to_string());
    });
}

#[test]
fn root_toplevel_nesting_list() {
    let (flake, _lock) = load_fixtures("toplevel_nesting");
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let info = Info::new("".into(), vec![]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_yaml_snapshot!(flake_edit.list());
    });
}
// #[test]
// fn root_alt_add_toplevel_id_uri() {
//     let (flake, _lock) = load_fixtures("root_alt");
//     let mut walker = Walker::new(&flake).unwrap();
//     let change = Change::Add {
//         id: Some("vmsh".to_owned()),
//         uri: Some("github:mic92/vmsh".to_owned()),
//     };
//     walker.changes.push(change.clone());
//     let info = Info::new("".into(), vec![change]);
//     let change = walker.walk().unwrap();
//     insta::with_settings!({sort_maps => true, info => &info}, {
//         insta::assert_snapshot!(change.to_string());
//     });
// }
// #[test]
// fn root_alt_remove_toplevel_uri() {
//     let (flake, _lock) = load_fixtures("root_alt");
//     let mut walker = Walker::new(&flake).unwrap();
//     let change = Change::Remove {
//         id: "nixpkgs".to_owned(),
//     };
//     walker.changes.push(change.clone());
//     let info = Info::new("".into(), vec![change]);
//     let change = walker.walk().unwrap();
//     insta::with_settings!({sort_maps => true, info => &info}, {
//         insta::assert_snapshot!(change.to_string());
//     });
// }
// #[test]
// fn root_alt_remove_toplevel_input_multiple() {
//     let (flake, _lock) = load_fixtures("root_alt");
//     let mut walker = Walker::new(&flake).unwrap();
//     let change = Change::Remove {
//         id: "crane".to_owned(),
//     };
//     walker.changes.push(change.clone());
//     let info = Info::new("".into(), vec![change]);
//     let change = walker.walk().unwrap();
//     insta::with_settings!({sort_maps => true, info => &info}, {
//         insta::assert_snapshot!(change.to_string());
//     });
// }

#[test]
fn completely_flat_toplevel_list() {
    let (flake, _lock) = load_fixtures("completely_flat_toplevel");
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let info = Info::new("".into(), vec![]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_yaml_snapshot!(flake_edit.list());
    });
}

#[test]
fn completely_flat_toplevel_alt_list() {
    let (flake, _lock) = load_fixtures("completely_flat_toplevel_alt");
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let info = Info::new("".into(), vec![]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_yaml_snapshot!(flake_edit.list());
    });
}
#[test]
fn completely_flat_toplevel_add_id_uri() {
    let (flake, _lock) = load_fixtures("completely_flat_toplevel");
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let change = Change::Add {
        id: Some("vmsh".to_owned()),
        uri: Some("mic92/vmsh".to_owned()),
        flake: true,
    };
    let info = Info::new("".into(), vec![change.clone()]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(flake_edit.apply_change(change).unwrap().unwrap());
    });
}
#[test]
fn completely_flat_toplevel_add_id_uri_no_flake() {
    let (flake, _lock) = load_fixtures("completely_flat_toplevel");
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let change = Change::Add {
        id: Some("not_a_flake".to_owned()),
        uri: Some("github:a-kenji/not_a_flake".to_owned()),
        flake: false,
    };
    let info = Info::new("".into(), vec![change.clone()]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(flake_edit.apply_change(change).unwrap().unwrap());
    });
}
#[test]
fn completely_flat_toplevel_rm_toplevel() {
    let (flake, _lock) = load_fixtures("completely_flat_toplevel");
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let change = Change::Remove {
        id: "nixpkgs".to_owned().into(),
    };
    let info = Info::new("".into(), vec![change.clone()]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(flake_edit.apply_change(change).unwrap().unwrap());
    });
}
#[test]
fn completely_flat_toplevel_rm_toplevel_multiple() {
    let (flake, _lock) = load_fixtures("completely_flat_toplevel");
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let change = Change::Remove {
        id: "crane".to_owned().into(),
    };
    let info = Info::new("".into(), vec![change.clone()]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(flake_edit.apply_change(change).unwrap().unwrap());
    });
}
#[test]
fn completely_flat_toplevel_rm_follows_single() {
    let (flake, _lock) = load_fixtures("completely_flat_toplevel");
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let change = Change::Remove {
        id: "crane.rust-overlay".to_owned().into(),
    };
    let info = Info::new("".into(), vec![change.clone()]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(flake_edit.apply_change(change).unwrap().unwrap());
    });
}
#[test]
fn completely_flat_toplevel_no_flake_rm_single_no_flake() {
    let (flake, _lock) = load_fixtures("completely_flat_toplevel_not_a_flake");
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let change = Change::Remove {
        id: "not-a-flake".to_owned().into(),
    };
    let info = Info::new("".into(), vec![change.clone()]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(flake_edit.apply_change(change).unwrap().unwrap());
    });
}
#[test]
#[should_panic]
fn completely_flat_toplevel_no_flake_rm_single_no_flake_rm_nonexistent() {
    let (flake, _lock) = load_fixtures("completely_flat_toplevel_not_a_flake");
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let change = Change::Remove {
        id: "not-an-input-at-all".to_owned().into(),
    };
    flake_edit.apply_change(change).unwrap().unwrap();
}
#[test]
fn completely_flat_toplevel_no_flake_list() {
    let (flake, _lock) = load_fixtures("completely_flat_toplevel_not_a_flake");
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let change = Change::None;
    let info = Info::new("".into(), vec![change.clone()]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_yaml_snapshot!(flake_edit.list());
    });
}
#[test]
fn completely_flat_toplevel_no_flake_rm_single_no_flake_nested() {
    let (flake, _lock) = load_fixtures("completely_flat_toplevel_not_a_flake_nested");
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let change = Change::Remove {
        id: "not-a-flake".to_owned().into(),
    };
    let info = Info::new("".into(), vec![change.clone()]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(flake_edit.apply_change(change).unwrap().unwrap());
    });
}
#[test]
fn completely_flat_toplevel_no_flake_nested_list() {
    let (flake, _lock) = load_fixtures("completely_flat_toplevel_not_a_flake_nested");
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let change = Change::None;
    let info = Info::new("".into(), vec![change.clone()]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_yaml_snapshot!(flake_edit.list());
    });
}
#[test]
fn one_level_nesting_flat_no_flake_rm_single_no_flake_nested() {
    let (flake, _lock) = load_fixtures("one_level_nesting_flat_not_a_flake");
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let change = Change::Remove {
        id: "not-a-flake".to_owned().into(),
    };
    let info = Info::new("".into(), vec![change.clone()]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(flake_edit.apply_change(change).unwrap().unwrap());
    });
}
#[test]
fn one_level_nesting_flat_no_flake_nested_list() {
    let (flake, _lock) = load_fixtures("one_level_nesting_flat_not_a_flake");
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let change = Change::None;
    let info = Info::new("".into(), vec![change.clone()]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_yaml_snapshot!(flake_edit.list());
    });
}
// #[test]
// fn root_alt_remove_toplevel_uri() {
//     let (flake, _lock) = load_fixtures("root_alt");
//     let mut walker = Walker::new(&flake).unwrap();
//     let change = Change::Remove {
//         id: "nixpkgs".to_owned(),
//     };
//     walker.changes.push(change.clone());
//     let info = Info::new("".into(), vec![change]);
//     let change = walker.walk().unwrap();
//     insta::with_settings!({sort_maps => true, info => &info}, {
//         insta::assert_snapshot!(change.to_string());
//     });
// }
// #[test]
// fn root_alt_remove_toplevel_input_multiple() {
//     let (flake, _lock) = load_fixtures("root_alt");
//     let mut walker = Walker::new(&flake).unwrap();
//     let change = Change::Remove {
//         id: "crane".to_owned(),
//     };
//     walker.changes.push(change.clone());
//     let info = Info::new("".into(), vec![change]);
//     let change = walker.walk().unwrap();
//     insta::with_settings!({sort_maps => true, info => &info}, {
//         insta::assert_snapshot!(change.to_string());
//     });
// }

#[test]
fn one_level_nesting_flat() {
    let (flake, _lock) = load_fixtures("one_level_nesting_flat");
    let mut walker = Walker::new(&flake);
    walker.walk(&Change::None);
    let info = Info::new("".into(), vec![]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_yaml_snapshot!(walker.inputs);
    });
}
#[test]
fn one_level_nesting_flat_remove_single() {
    let (flake, _lock) = load_fixtures("one_level_nesting_flat");
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let change = Change::Remove {
        id: "nixpkgs".to_owned().into(),
    };
    let info = Info::new("".into(), vec![]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(flake_edit.apply_change(change).unwrap().unwrap());
    });
}
#[test]
fn one_level_nesting_flat_remove_multiple() {
    let (flake, _lock) = load_fixtures("one_level_nesting_flat");
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let change = Change::Remove {
        id: "rust-overlay".to_owned().into(),
    };
    let info = Info::new("".into(), vec![]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(flake_edit.apply_change(change).unwrap().unwrap());
    });
}
#[test]
fn one_level_nesting_flat_remove_single_nested() {
    let (flake, _lock) = load_fixtures("one_level_nesting_flat");
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let change = Change::Remove {
        id: "rust-overlay.flake-utils".to_owned().into(),
    };
    let info = Info::new("".into(), vec![]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(flake_edit.apply_change(change).unwrap().unwrap());
    });
}

#[test]
fn flat_nested_flat_remove_single() {
    let (flake, _lock) = load_fixtures("flat_nested_flat");
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let change = Change::Remove {
        id: "nixpkgs".to_owned().into(),
    };
    let info = Info::new("".into(), vec![]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(flake_edit.apply_change(change).unwrap().unwrap());
    });
}
#[test]
fn flat_nested_flat_remove_multiple() {
    let (flake, _lock) = load_fixtures("flat_nested_flat");
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let change = Change::Remove {
        id: "poetry2nix".to_owned().into(),
    };
    let info = Info::new("".into(), vec![]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(flake_edit.apply_change(change).unwrap().unwrap());
    });
}
#[test]
fn flat_nested_flat_add_single() {
    let (flake, _lock) = load_fixtures("flat_nested_flat");
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let change = Change::Add {
        id: Some("vmsh".to_owned()),
        uri: Some("mic92/vmsh".to_owned()),
        flake: true,
    };
    let info = Info::new("".into(), vec![]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(flake_edit.apply_change(change).unwrap().unwrap());
    });
}
#[test]
fn flat_nested_flat_add_single_no_flake() {
    let (flake, _lock) = load_fixtures("flat_nested_flat");
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let change = Change::Add {
        id: Some("not_a_flake".to_owned()),
        uri: Some("github:a-kenji/not_a_flake".to_owned()),
        flake: false,
    };
    let info = Info::new("".into(), vec![]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(flake_edit.apply_change(change).unwrap().unwrap());
    });
}
#[test]
fn first_nested_node_add_single() {
    let (flake, _lock) = load_fixtures("first_nested_node");
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let change = Change::Add {
        id: Some("vmsh".to_owned()),
        uri: Some("mic92/vmsh".to_owned()),
        flake: true,
    };
    let info = Info::new("".into(), vec![]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!("changes", flake_edit.apply_change(change.clone()).unwrap().unwrap());
    });
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_yaml_snapshot!("list", flake_edit.curr_list());
    });
}
#[test]
fn first_nested_node_add_single_no_flake() {
    let (flake, _lock) = load_fixtures("first_nested_node");
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let change = Change::Add {
        id: Some("vmsh".to_owned()),
        uri: Some("mic92/vmsh".to_owned()),
        flake: true,
    };
    let info = Info::new("".into(), vec![]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!("changes", flake_edit.apply_change(change.clone()).unwrap().unwrap());
    });
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_yaml_snapshot!("list", flake_edit.curr_list());
    });
}
#[test]
fn first_nested_node_remove_single() {
    let (flake, _lock) = load_fixtures("first_nested_node");
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let change = Change::Remove {
        id: "utils".to_string().into(),
    };
    let info = Info::new("".into(), vec![change.clone()]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(flake_edit.apply_change(change.clone()).unwrap().unwrap());
    });
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_yaml_snapshot!(flake_edit.list());
    });
}
#[test]
fn first_nested_node_remove_multiple() {
    let (flake, _lock) = load_fixtures("first_nested_node");
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let change = Change::Remove {
        id: "naersk".to_string().into(),
    };
    let info = Info::new("".into(), vec![change.clone()]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(flake_edit.apply_change(change.clone()).unwrap().unwrap());
    });
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_yaml_snapshot!(flake_edit.list());
    });
}
#[test]
fn first_nested_node_inputs() {
    let (flake, _lock) = load_fixtures("first_nested_node");
    let mut walker = Walker::new(&flake);
    walker.walk(&Change::None);
    let info = Info::new("".into(), vec![]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_yaml_snapshot!(walker.inputs);
    });
}

#[test]
fn toggle_test_rust_overlay() {
    let (flake, _lock) = load_fixtures("toggle_test");
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let change = Change::Toggle {
        id: Some("rust-overlay".to_owned()),
    };
    let info = Info::new("".into(), vec![change.clone()]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(flake_edit.apply_change(change).unwrap().unwrap());
    });
}

#[test]
fn toggle_test_auto_detect() {
    let (flake, _lock) = load_fixtures("toggle_test");
    let mut flake_edit = FlakeEdit::from_text(&flake).unwrap();
    let change = Change::Toggle {
        id: None, // Auto-detect
    };
    let info = Info::new("".into(), vec![change.clone()]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(flake_edit.apply_change(change).unwrap().unwrap());
    });
}

// #[test]
// fn root_alt_add_toplevel_id_uri() {
//     let (flake, _lock) = load_fixtures("root_alt");
//     let mut walker = Walker::new(&flake).unwrap();
//     let change = Change::Add {
//         id: Some("vmsh".to_owned()),
//         uri: Some("github:mic92/vmsh".to_owned()),
//     };
//     walker.changes.push(change.clone());
//     let info = Info::new("".into(), vec![change]);
//     let change = walker.walk().unwrap();
//     insta::with_settings!({sort_maps => true, info => &info}, {
//         insta::assert_snapshot!(change.to_string());
//     });
// }
// #[test]
// fn root_alt_remove_toplevel_uri() {
//     let (flake, _lock) = load_fixtures("root_alt");
//     let mut walker = Walker::new(&flake).unwrap();
//     let change = Change::Remove {
//         id: "nixpkgs".to_owned(),
//     };
//     walker.changes.push(change.clone());
//     let info = Info::new("".into(), vec![change]);
//     let change = walker.walk().unwrap();
//     insta::with_settings!({sort_maps => true, info => &info}, {
//         insta::assert_snapshot!(change.to_string());
//     });
// }
// #[test]
// fn root_alt_remove_toplevel_input_multiple() {
//     let (flake, _lock) = load_fixtures("root_alt");
//     let mut walker = Walker::new(&flake).unwrap();
//     let change = Change::Remove {
//         id: "crane".to_owned(),
//     };
//     walker.changes.push(change.clone());
//     let info = Info::new("".into(), vec![change]);
//     let change = walker.walk().unwrap();
//     insta::with_settings!({sort_maps => true, info => &info}, {
//         insta::assert_snapshot!(change.to_string());
//     });
// }
