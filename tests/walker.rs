use flake_edit::walk::Walker;
use flake_edit::Change;

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
fn root_list() {
    let (flake, _lock) = load_fixtures("root");
    let mut walker = Walker::new(&flake).unwrap();
    walker.walk();
    let info = Info::new("".into(), vec![]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_yaml_snapshot!(walker.inputs);
    });
}
#[test]
fn root_add_toplevel_id_uri() {
    let (flake, _lock) = load_fixtures("root");
    let mut walker = Walker::new(&flake).unwrap();
    let change = Change::Add {
        id: Some("vmsh".to_owned()),
        uri: Some("github:mic92/vmsh".to_owned()),
    };
    walker.changes.push(change.clone());
    let info = Info::new("".into(), vec![change]);
    let change = walker.walk().unwrap();
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(change.to_string());
    });
}
#[test]
fn root_remove_toplevel_uri() {
    let (flake, _lock) = load_fixtures("root");
    let mut walker = Walker::new(&flake).unwrap();
    let change = Change::Remove {
        id: "nixpkgs".to_owned(),
    };
    walker.changes.push(change.clone());
    let info = Info::new("".into(), vec![change]);
    let change = walker.walk().unwrap();
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(change.to_string());
    });
}
#[test]
fn root_remove_toplevel_input_multiple() {
    let (flake, _lock) = load_fixtures("root");
    let mut walker = Walker::new(&flake).unwrap();
    let change = Change::Remove {
        id: "crane".to_owned(),
    };
    walker.changes.push(change.clone());
    let info = Info::new("".into(), vec![change]);
    let change = walker.walk().unwrap();
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(change.to_string());
    });
}
#[test]
fn root_alt_list() {
    let (flake, _lock) = load_fixtures("root_alt");
    let mut walker = Walker::new(&flake).unwrap();
    walker.walk();
    let info = Info::new("".into(), vec![]);
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_yaml_snapshot!(walker.inputs);
    });
}
#[test]
fn root_alt_add_toplevel_id_uri() {
    let (flake, _lock) = load_fixtures("root_alt");
    let mut walker = Walker::new(&flake).unwrap();
    let change = Change::Add {
        id: Some("vmsh".to_owned()),
        uri: Some("github:mic92/vmsh".to_owned()),
    };
    walker.changes.push(change.clone());
    let info = Info::new("".into(), vec![change]);
    let change = walker.walk().unwrap();
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(change.to_string());
    });
}
#[test]
fn root_alt_remove_toplevel_uri() {
    let (flake, _lock) = load_fixtures("root_alt");
    let mut walker = Walker::new(&flake).unwrap();
    let change = Change::Remove {
        id: "nixpkgs".to_owned(),
    };
    walker.changes.push(change.clone());
    let info = Info::new("".into(), vec![change]);
    let change = walker.walk().unwrap();
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(change.to_string());
    });
}
#[test]
fn root_alt_remove_toplevel_input_multiple() {
    let (flake, _lock) = load_fixtures("root_alt");
    let mut walker = Walker::new(&flake).unwrap();
    let change = Change::Remove {
        id: "crane".to_owned(),
    };
    walker.changes.push(change.clone());
    let info = Info::new("".into(), vec![change]);
    let change = walker.walk().unwrap();
    insta::with_settings!({sort_maps => true, info => &info}, {
        insta::assert_snapshot!(change.to_string());
    });
}

#[test]
fn root_toplevel_nesting_list() {
    let (flake, _lock) = load_fixtures("toplevel_nesting");
    let mut walker = Walker::new(&flake).unwrap();
    walker.walk();
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
