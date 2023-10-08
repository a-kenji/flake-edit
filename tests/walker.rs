use flake_edit::walk::Walker;

fn load_fixtures(name: &str) -> (String, String) {
    let dir = env!("CARGO_MANIFEST_DIR");
    let flake_nix =
        std::fs::read_to_string(format!("{dir}/tests/fixtures/{name}.flake.nix")).unwrap();
    let flake_lock =
        std::fs::read_to_string(format!("{dir}/tests/fixtures/{name}.flake.lock")).unwrap();
    (flake_nix, flake_lock)
}

#[test]
fn load_root() {
    let (flake, _lock) = load_fixtures("root");
    let mut walker = Walker::new(&flake).unwrap();
    walker.walk_toplevel();
    insta::with_settings!({sort_maps => true}, {
        insta::assert_yaml_snapshot!(walker.inputs);
    });
}
