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
    let mut flake_edit = FlakeEdit::from(&flake).unwrap();
    let info = Info::new("".into(), vec![]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_yaml_snapshot!(flake_edit.list());
    });
}
#[test]
fn root_add_toplevel_id_uri() {
    let (flake, _lock) = load_fixtures("root");
    let mut flake_edit = FlakeEdit::from(&flake).unwrap();
    let change = Change::Add {
        id: Some("vmsh".to_owned()),
        uri: Some("github:mic92/vmsh".to_owned()),
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
        id: "nixpkgs".to_owned(),
    };
    let info = Info::new("".into(), vec![change.clone()]);
    let change = walker.walk(&change).unwrap();
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(change.to_string());
    });
}
// #[test]
// fn root_remove_toplevel_input_multiple() {
//     let (flake, _lock) = load_fixtures("root");
//     let mut walker = Walker::new(&flake);
//     let change = Change::Remove {
//         id: "crane".to_owned(),
//     };
//     let info = Info::new("".into(), vec![change.clone()]);
//     let change = walker.walk(&change).unwrap();
//     insta::with_settings!({sort_maps => true, info => &info}, {
//         insta::assert_snapshot!(change.to_string());
//     });
// }
#[test]
fn root_alt_list() {
    let (flake, _lock) = load_fixtures("root_alt");
    let mut flake_edit = FlakeEdit::from(&flake).unwrap();
    let info = Info::new("".into(), vec![]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_yaml_snapshot!(flake_edit.list());
    });
}
// #[test]
// fn root_alt_add_toplevel_id_uri() {
//     let (flake, _lock) = load_fixtures("root_alt");
//     let mut walker = Walker::new(&flake);
//     let change = Change::Add {
//         id: Some("vmsh".to_owned()),
//         uri: Some("github:mic92/vmsh".to_owned()),
//     };
//     let info = Info::new("".into(), vec![change.clone()]);
//     let change = walker.walk(&change).unwrap();
//     insta::with_settings!({sort_maps => true, info => &info}, {
//         insta::assert_snapshot!(change.to_string());
//     });
// }
// #[test]
// fn root_alt_remove_toplevel_uri() {
//     let (flake, _lock) = load_fixtures("root_alt");
//     let mut walker = Walker::new(&flake);
//     let change = Change::Remove {
//         id: "nixpkgs".to_owned(),
//     };
//     let info = Info::new("".into(), vec![change.clone()]);
//     let change = walker.walk(&change).unwrap();
//     insta::with_settings!({sort_maps => true, info => &info}, {
//         insta::assert_snapshot!(change.to_string());
//     });
// }
// #[test]
// fn root_alt_remove_toplevel_input_multiple() {
//     let (flake, _lock) = load_fixtures("root_alt");
//     let mut walker = Walker::new(&flake);
//     let change = Change::Remove {
//         id: "crane".to_owned(),
//     };
//     let info = Info::new("".into(), vec![change.clone()]);
//     let change = walker.walk(&change).unwrap();
//     insta::with_settings!({sort_maps => true, info => &info}, {
//         insta::assert_snapshot!(change.to_string());
//     });
// }
//
#[test]
fn root_toplevel_nesting_list() {
    let (flake, _lock) = load_fixtures("toplevel_nesting");
    let mut flake_edit = FlakeEdit::from(&flake).unwrap();
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
    let mut flake_edit = FlakeEdit::from(&flake).unwrap();
    let info = Info::new("".into(), vec![]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_yaml_snapshot!(flake_edit.list());
    });
}

#[test]
fn completely_flat_toplevel_alt_list() {
    let (flake, _lock) = load_fixtures("completely_flat_toplevel_alt");
    let mut flake_edit = FlakeEdit::from(&flake).unwrap();
    let info = Info::new("".into(), vec![]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_yaml_snapshot!(flake_edit.list());
    });
}
#[test]
fn completely_flat_toplevel_add_id_uri() {
    let (flake, _lock) = load_fixtures("completely_flat_toplevel");
    let mut flake_edit = FlakeEdit::from(&flake).unwrap();
    let change = Change::Add {
        id: Some("vmsh".to_owned()),
        uri: Some("mic92/vmsh".to_owned()),
    };
    let info = Info::new("".into(), vec![change.clone()]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(flake_edit.apply_change(change).unwrap().unwrap());
    });
}
#[test]
fn completely_flat_toplevel_rm_toplevel() {
    let (flake, _lock) = load_fixtures("completely_flat_toplevel");
    let mut flake_edit = FlakeEdit::from(&flake).unwrap();
    let change = Change::Remove {
        id: "nixpkgs".to_owned(),
    };
    let info = Info::new("".into(), vec![change.clone()]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(flake_edit.apply_change(change).unwrap().unwrap());
    });
}
#[test]
fn completely_flat_toplevel_rm_toplevel_muliple() {
    let (flake, _lock) = load_fixtures("completely_flat_toplevel");
    let mut flake_edit = FlakeEdit::from(&flake).unwrap();
    let change = Change::Remove {
        id: "crane".to_owned(),
    };
    let info = Info::new("".into(), vec![change.clone()]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(flake_edit.apply_change(change).unwrap().unwrap());
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
