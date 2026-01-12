//! Integration tests for the TUI module
//!
//! These tests verify the TUI app rendering and state transitions using
//! ratatui's TestBackend for deterministic snapshot testing.
//!
//! Tests use `App::from_command` to create apps from CLI commands,
//! ensuring we test the same code path as the real application.
//!
//! Fixtures are loaded from `tests/fixtures/` and inputs are extracted
//! using the same parsing logic as the main application.

#![cfg(feature = "tui")]

use clap::Parser;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use flake_edit::cli::CliArgs;
use flake_edit::edit::FlakeEdit;
use flake_edit::tui::App;
use flake_edit::tui::app::UpdateResult;
use ratatui::{Terminal, backend::TestBackend, widgets::Widget};
use rstest::rstest;

/// Load a fixture file from the fixtures directory
fn load_fixture(name: &str) -> String {
    let dir = env!("CARGO_MANIFEST_DIR");
    let path = format!("{dir}/tests/fixtures/{name}.flake.nix");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("Failed to load fixture {path}: {e}"))
}

/// Extract inputs from a flake text as (id, uri) pairs, sorted by id for determinism
fn extract_inputs(flake_text: &str) -> Vec<(String, String)> {
    let mut edit = FlakeEdit::from_text(flake_text).expect("Failed to parse flake");
    let mut inputs: Vec<_> = edit
        .list()
        .iter()
        .map(|(id, input)| (id.clone(), input.url().trim_matches('"').to_string()))
        .collect();
    inputs.sort_by(|a, b| a.0.cmp(&b.0));
    inputs
}

/// A test fixture containing flake text and extracted inputs
struct Fixture {
    text: String,
    inputs: Vec<(String, String)>,
}

impl Fixture {
    fn load(name: &str) -> Self {
        let text = load_fixture(name);
        let inputs = extract_inputs(&text);
        Self { text, inputs }
    }
}

/// Parse CLI arguments and create App using from_command with a fixture
fn app_from_args_with_fixture(args: &str, fixture: &Fixture) -> Option<App> {
    let args: Vec<&str> = std::iter::once("flake-edit")
        .chain(args.split_whitespace())
        .collect();
    let cli = CliArgs::try_parse_from(args).expect("Failed to parse CLI args");
    App::from_command(
        cli.subcommand(),
        &fixture.text,
        fixture.inputs.clone(),
        cli.diff(),
    )
}

/// Create a test terminal with the given dimensions
fn create_test_terminal(width: u16, height: u16) -> Terminal<TestBackend> {
    let backend = TestBackend::new(width, height);
    Terminal::new(backend).unwrap()
}

/// Render app to terminal and return snapshot
fn snapshot(terminal: &mut Terminal<TestBackend>, app: &App) -> String {
    terminal
        .draw(|frame| {
            app.render(frame.area(), frame.buffer_mut());
        })
        .unwrap();
    format!("{:?}", terminal.backend().buffer())
}

/// Create a KeyEvent for testing
fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// Create a KeyEvent with Ctrl modifier
fn ctrl_key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

/// Test session that tracks actions for snapshot descriptions
struct TestSession {
    app: App,
    actions: Vec<String>,
}

impl TestSession {
    fn new(app: App, initial: impl Into<String>) -> Self {
        Self {
            app,
            actions: vec![initial.into()],
        }
    }

    fn type_text(&mut self, text: &str) {
        self.actions.push(format!("type '{text}'"));
        for c in text.chars() {
            self.app.update(key(KeyCode::Char(c)));
        }
    }

    fn submit(&mut self) -> UpdateResult {
        self.actions.push("Enter".to_string());
        self.app.update(key(KeyCode::Enter))
    }

    fn escape(&mut self) -> UpdateResult {
        self.actions.push("Esc".to_string());
        self.app.update(key(KeyCode::Esc))
    }

    fn nav_down(&mut self) {
        self.actions.push("Down".to_string());
        self.app.update(key(KeyCode::Down));
    }

    fn toggle_select(&mut self) {
        self.actions.push("Space".to_string());
        self.app.update(key(KeyCode::Char(' ')));
    }

    fn toggle_all(&mut self) {
        self.actions.push("u".to_string());
        self.app.update(key(KeyCode::Char('u')));
    }

    fn ctrl(&mut self, c: char) {
        self.actions.push(format!("Ctrl+{c}"));
        self.app.update(ctrl_key(c));
    }

    fn press(&mut self, c: char) {
        self.actions.push(c.to_string());
        self.app.update(key(KeyCode::Char(c)));
    }

    fn backspace(&mut self) {
        self.actions.push("Backspace".to_string());
        self.app.update(key(KeyCode::Backspace));
    }

    fn description(&self) -> String {
        self.actions.join(" → ")
    }

    fn app(&self) -> &App {
        &self.app
    }
}

#[rstest]
#[case("root")]
#[case("root_alt")]
#[case("completely_flat_toplevel")]
fn test_add_interactive_creates_app(#[case] fixture_name: &str) {
    let fixture = Fixture::load(fixture_name);
    let app = app_from_args_with_fixture("add", &fixture);
    assert!(app.is_some());
}

#[rstest]
#[case("root")]
fn test_add_with_all_args_no_app(#[case] fixture_name: &str) {
    let fixture = Fixture::load(fixture_name);
    let app = app_from_args_with_fixture("add nixpkgs github:nixos/nixpkgs", &fixture);
    assert!(app.is_none());
}

#[rstest]
#[case("root")]
#[case("completely_flat_toplevel")]
fn test_remove_interactive_creates_app(#[case] fixture_name: &str) {
    let fixture = Fixture::load(fixture_name);
    let app = app_from_args_with_fixture("rm", &fixture);
    assert!(app.is_some());
}

#[rstest]
#[case("root")]
fn test_change_interactive_creates_list(#[case] fixture_name: &str) {
    let fixture = Fixture::load(fixture_name);
    let app = app_from_args_with_fixture("change", &fixture);
    assert!(app.is_some());
}

#[rstest]
#[case("root", "nixpkgs")]
#[case("completely_flat_toplevel", "nixpkgs")]
fn test_change_with_id_creates_input(#[case] fixture_name: &str, #[case] input_id: &str) {
    let fixture = Fixture::load(fixture_name);
    let app = app_from_args_with_fixture(&format!("change {input_id}"), &fixture);
    assert!(app.is_some());
}

#[rstest]
#[case("root")]
fn test_pin_interactive_creates_app(#[case] fixture_name: &str) {
    let fixture = Fixture::load(fixture_name);
    let app = app_from_args_with_fixture("pin", &fixture);
    assert!(app.is_some());
}

#[rstest]
#[case("root")]
fn test_update_interactive_creates_app(#[case] fixture_name: &str) {
    let fixture = Fixture::load(fixture_name);
    let app = app_from_args_with_fixture("update", &fixture);
    assert!(app.is_some());
}

#[rstest]
#[case("root")]
fn test_list_no_app(#[case] fixture_name: &str) {
    let fixture = Fixture::load(fixture_name);
    let app = app_from_args_with_fixture("list", &fixture);
    assert!(app.is_none());
}

#[rstest]
#[case("root")]
#[case("root_alt")]
#[case("completely_flat_toplevel")]
#[case("toplevel_nesting")]
fn test_add_initial_screen(#[case] fixture_name: &str) {
    let fixture = Fixture::load(fixture_name);
    let mut terminal = create_test_terminal(80, 4);
    let app = app_from_args_with_fixture("add", &fixture).unwrap();
    insta::with_settings!({
        snapshot_suffix => fixture_name
    }, {
        insta::assert_snapshot!(snapshot(&mut terminal, &app));
    });
}

#[rstest]
#[case("root")]
#[case("root_alt")]
#[case("completely_flat_toplevel")]
fn test_remove_initial_screen(#[case] fixture_name: &str) {
    let fixture = Fixture::load(fixture_name);
    let height = (fixture.inputs.len() as u16 + 3).min(12);
    let mut terminal = create_test_terminal(80, height);
    let app = app_from_args_with_fixture("rm", &fixture).unwrap();
    insta::with_settings!({
        snapshot_suffix => fixture_name
    }, {
        insta::assert_snapshot!(snapshot(&mut terminal, &app));
    });
}

#[rstest]
#[case("root")]
#[case("root_alt")]
#[case("completely_flat_toplevel")]
fn test_change_list_screen(#[case] fixture_name: &str) {
    let fixture = Fixture::load(fixture_name);
    let height = (fixture.inputs.len() as u16 + 3).min(12);
    let mut terminal = create_test_terminal(80, height);
    let app = app_from_args_with_fixture("change", &fixture).unwrap();
    insta::with_settings!({
        snapshot_suffix => fixture_name
    }, {
        insta::assert_snapshot!(snapshot(&mut terminal, &app));
    });
}

#[rstest]
#[case("root", "nixpkgs")]
#[case("completely_flat_toplevel", "nixpkgs")]
fn test_change_uri_screen(#[case] fixture_name: &str, #[case] input_id: &str) {
    let fixture = Fixture::load(fixture_name);
    let mut terminal = create_test_terminal(80, 4);
    let app = app_from_args_with_fixture(&format!("change {input_id}"), &fixture).unwrap();
    insta::with_settings!({
        snapshot_suffix => format!("{fixture_name}_{input_id}")
    }, {
        insta::assert_snapshot!(snapshot(&mut terminal, &app));
    });
}

#[rstest]
#[case("root")]
#[case("completely_flat_toplevel")]
fn test_pin_initial_screen(#[case] fixture_name: &str) {
    let fixture = Fixture::load(fixture_name);
    let height = (fixture.inputs.len() as u16 + 3).min(12);
    let mut terminal = create_test_terminal(80, height);
    let app = app_from_args_with_fixture("pin", &fixture).unwrap();
    insta::with_settings!({
        snapshot_suffix => fixture_name
    }, {
        insta::assert_snapshot!(snapshot(&mut terminal, &app));
    });
}

#[rstest]
#[case("root")]
#[case("completely_flat_toplevel")]
fn test_update_initial_screen(#[case] fixture_name: &str) {
    let fixture = Fixture::load(fixture_name);
    let height = (fixture.inputs.len() as u16 + 3).min(12);
    let mut terminal = create_test_terminal(80, height);
    let app = app_from_args_with_fixture("update", &fixture).unwrap();
    insta::with_settings!({
        snapshot_suffix => fixture_name
    }, {
        insta::assert_snapshot!(snapshot(&mut terminal, &app));
    });
}

#[rstest]
#[case("root")]
#[case("completely_flat_toplevel")]
fn test_diff_flag_passed_to_app(#[case] fixture_name: &str) {
    let fixture = Fixture::load(fixture_name);
    let app = app_from_args_with_fixture("--diff pin", &fixture).unwrap();
    assert!(app.show_diff());
    // Use fixed height large enough for list + diff preview
    let height = (fixture.inputs.len() as u16 + 3).min(12);
    let mut terminal = create_test_terminal(80, height);
    insta::with_settings!({
        snapshot_suffix => fixture_name
    }, {
        insta::assert_snapshot!(snapshot(&mut terminal, &app));
    });
}

#[rstest]
#[case("root")]
#[case("completely_flat_toplevel")]
fn test_add_workflow_complete(#[case] fixture_name: &str) {
    let fixture = Fixture::load(fixture_name);
    let mut terminal = create_test_terminal(80, 4);
    let app = app_from_args_with_fixture("add", &fixture).unwrap();
    let mut session = TestSession::new(app, "add");

    // Snapshot initial URI input screen
    insta::with_settings!({
        snapshot_suffix => format!("{fixture_name}_1_uri_input"),
        description => session.description()
    }, {
        insta::assert_snapshot!(snapshot(&mut terminal, session.app()));
    });

    // Enter URI and submit
    session.type_text("github:user/repo");
    let result = session.submit();
    assert!(matches!(result, UpdateResult::Continue));

    // Snapshot ID input screen (with inferred ID prefilled)
    insta::with_settings!({
        snapshot_suffix => format!("{fixture_name}_2_id_input"),
        description => session.description()
    }, {
        insta::assert_snapshot!(snapshot(&mut terminal, session.app()));
    });

    // Clear auto-inferred and enter custom ID
    session.ctrl('u');
    session.type_text("my-input");
    let result = session.submit();
    assert!(matches!(result, UpdateResult::Done));

    // Snapshot the resulting diff
    insta::with_settings!({
        snapshot_suffix => format!("{fixture_name}_3_result_diff"),
        description => session.description()
    }, {
        insta::assert_snapshot!(session.app().pending_diff());
    });
}

#[rstest]
#[case("root")]
fn test_add_workflow_with_diff_shows_confirm(#[case] fixture_name: &str) {
    let fixture = Fixture::load(fixture_name);
    let mut terminal = create_test_terminal(80, 10);
    let app = app_from_args_with_fixture("--diff add", &fixture).unwrap();
    let mut session = TestSession::new(app, "--diff add");

    // Enter URI and ID
    session.type_text("github:user/new-input");
    session.submit();
    session.ctrl('u');
    session.type_text("new-input");
    let result = session.submit();

    // With diff mode, should show confirm screen
    assert!(matches!(result, UpdateResult::Continue));
    insta::with_settings!({
        snapshot_suffix => fixture_name,
        description => session.description()
    }, {
        insta::assert_snapshot!(snapshot(&mut terminal, session.app()));
    });
}

/// Test that diff preview shows actual changes during Add workflow input
#[rstest]
#[case("root")]
fn test_add_workflow_diff_preview_during_input(#[case] fixture_name: &str) {
    let fixture = Fixture::load(fixture_name);
    let app = app_from_args_with_fixture("add", &fixture).unwrap();
    let mut session = TestSession::new(app, "add");

    // Enter URI and go to ID input
    session.type_text("github:user/my-new-input");
    session.submit();

    // Enter an ID
    session.ctrl('u'); // Clear auto-inferred
    session.type_text("my-new-input");

    // Toggle diff preview on
    session.ctrl('d');
    assert!(session.app().show_diff());

    // Use height large enough for input + diff preview (input=4 + diff~13)
    let mut terminal = create_test_terminal(80, 17);
    insta::with_settings!({
        snapshot_suffix => fixture_name,
        description => session.description()
    }, {
        insta::assert_snapshot!(snapshot(&mut terminal, session.app()));
    });
}

#[rstest]
#[case("root")]
fn test_add_workflow_cancel(#[case] fixture_name: &str) {
    let fixture = Fixture::load(fixture_name);
    let app = app_from_args_with_fixture("add", &fixture).unwrap();
    let mut session = TestSession::new(app, "add");
    let result = session.escape();
    assert!(matches!(result, UpdateResult::Cancelled));
}

#[rstest]
#[case("root")]
fn test_add_workflow_back_from_id(#[case] fixture_name: &str) {
    let fixture = Fixture::load(fixture_name);
    let mut terminal = create_test_terminal(80, 4);
    let app = app_from_args_with_fixture("add", &fixture).unwrap();
    let mut session = TestSession::new(app, "add");

    session.type_text("github:nixos/nixpkgs");
    session.submit();

    // Go back from ID screen
    let result = session.escape();
    assert!(matches!(result, UpdateResult::Continue));

    insta::with_settings!({
        snapshot_suffix => fixture_name,
        description => session.description()
    }, {
        insta::assert_snapshot!(snapshot(&mut terminal, session.app()));
    });
}

#[rstest]
#[case("root")]
#[case("completely_flat_toplevel")]
fn test_remove_workflow_complete(#[case] fixture_name: &str) {
    let fixture = Fixture::load(fixture_name);
    let app = app_from_args_with_fixture("rm", &fixture).unwrap();
    let mut session = TestSession::new(app, "rm");

    // Toggle first two items (or just first if only one)
    session.toggle_select();
    if fixture.inputs.len() > 1 {
        session.toggle_select();
    }

    let result = session.submit();
    assert!(matches!(result, UpdateResult::Done));

    insta::with_settings!({
        snapshot_suffix => fixture_name,
        description => session.description()
    }, {
        insta::assert_snapshot!(session.app().pending_diff());
    });
}

#[rstest]
#[case("root")]
fn test_change_workflow_select_then_input(#[case] fixture_name: &str) {
    let fixture = Fixture::load(fixture_name);
    let height = (fixture.inputs.len() as u16 + 3).min(12);
    let mut list_terminal = create_test_terminal(80, height);
    let mut input_terminal = create_test_terminal(80, 4);
    let app = app_from_args_with_fixture("change", &fixture).unwrap();
    let mut session = TestSession::new(app, "change");

    // Snapshot initial list screen
    insta::with_settings!({
        snapshot_suffix => format!("{fixture_name}_list"),
        description => session.description()
    }, {
        insta::assert_snapshot!(snapshot(&mut list_terminal, session.app()));
    });

    // Select first item
    let result = session.submit();
    assert!(matches!(result, UpdateResult::Continue));

    // Snapshot input screen after selection
    insta::with_settings!({
        snapshot_suffix => format!("{fixture_name}_input"),
        description => session.description()
    }, {
        insta::assert_snapshot!(snapshot(&mut input_terminal, session.app()));
    });
}

#[rstest]
#[case("root")]
#[case("completely_flat_toplevel")]
fn test_change_workflow_complete(#[case] fixture_name: &str) {
    let fixture = Fixture::load(fixture_name);
    let app = app_from_args_with_fixture("change", &fixture).unwrap();
    let mut session = TestSession::new(app, "change");

    // Select first item and enter new URI
    session.submit();
    session.ctrl('u');
    session.type_text("github:nixos/nixpkgs/nixos-unstable");
    let result = session.submit();

    assert!(matches!(result, UpdateResult::Done));

    insta::with_settings!({
        snapshot_suffix => fixture_name,
        description => session.description()
    }, {
        insta::assert_snapshot!(session.app().pending_diff());
    });
}

#[rstest]
#[case("root", "nixpkgs")]
fn test_change_uri_workflow(#[case] fixture_name: &str, #[case] input_id: &str) {
    let fixture = Fixture::load(fixture_name);
    let cmd = format!("change {input_id}");
    let app = app_from_args_with_fixture(&cmd, &fixture).unwrap();
    let mut session = TestSession::new(app, cmd);

    // Clear and enter new URI
    session.ctrl('u');
    session.type_text("github:nixos/nixpkgs/nixos-24.11");
    let result = session.submit();

    assert!(matches!(result, UpdateResult::Done));

    insta::with_settings!({
        snapshot_suffix => format!("{fixture_name}_{input_id}"),
        description => session.description()
    }, {
        insta::assert_snapshot!(session.app().pending_diff());
    });
}

#[rstest]
#[case("root")]
fn test_pin_workflow(#[case] fixture_name: &str) {
    let fixture = Fixture::load(fixture_name);
    let app = app_from_args_with_fixture("pin", &fixture).unwrap();
    let mut session = TestSession::new(app, "pin");

    // Navigate to second item and select
    session.nav_down();
    let result = session.submit();

    assert!(matches!(result, UpdateResult::Done));
}

#[rstest]
#[case("root")]
#[case("completely_flat_toplevel")]
fn test_update_workflow_select_all(#[case] fixture_name: &str) {
    let fixture = Fixture::load(fixture_name);
    let app = app_from_args_with_fixture("update", &fixture).unwrap();
    let mut session = TestSession::new(app, "update");

    // Toggle all with 'u'
    session.toggle_all();
    let result = session.submit();

    assert!(matches!(result, UpdateResult::Done));
}

#[rstest]
#[case("root")]
fn test_list_navigation_down(#[case] fixture_name: &str) {
    let fixture = Fixture::load(fixture_name);
    let height = (fixture.inputs.len() as u16 + 3).min(12);
    let mut terminal = create_test_terminal(80, height);
    let app = app_from_args_with_fixture("pin", &fixture).unwrap();
    let mut session = TestSession::new(app, "pin");

    session.nav_down();
    session.nav_down();

    insta::with_settings!({
        snapshot_suffix => fixture_name,
        description => session.description()
    }, {
        insta::assert_snapshot!(snapshot(&mut terminal, session.app()));
    });
}

#[rstest]
#[case("root")]
fn test_list_navigation_vim_keys(#[case] fixture_name: &str) {
    let fixture = Fixture::load(fixture_name);
    let height = (fixture.inputs.len() as u16 + 3).min(12);
    let mut terminal = create_test_terminal(80, height);
    let app = app_from_args_with_fixture("pin", &fixture).unwrap();
    let mut session = TestSession::new(app, "pin");

    session.press('j'); // down
    session.press('j'); // down
    session.press('k'); // up

    insta::with_settings!({
        snapshot_suffix => fixture_name,
        description => session.description()
    }, {
        insta::assert_snapshot!(snapshot(&mut terminal, session.app()));
    });
}

#[rstest]
#[case("root")]
fn test_list_wrap_around(#[case] fixture_name: &str) {
    let fixture = Fixture::load(fixture_name);
    let height = (fixture.inputs.len() as u16 + 3).min(12);
    let mut terminal = create_test_terminal(80, height);
    let app = app_from_args_with_fixture("pin", &fixture).unwrap();
    let mut session = TestSession::new(app, "pin");

    // Navigate past end
    for _ in 0..fixture.inputs.len() + 1 {
        session.nav_down();
    }

    insta::with_settings!({
        snapshot_suffix => fixture_name,
        description => session.description()
    }, {
        insta::assert_snapshot!(snapshot(&mut terminal, session.app()));
    });
}

#[rstest]
#[case("root")]
fn test_multi_select_toggle(#[case] fixture_name: &str) {
    let fixture = Fixture::load(fixture_name);
    let height = (fixture.inputs.len() as u16 + 3).min(12);
    let mut terminal = create_test_terminal(80, height);
    let app = app_from_args_with_fixture("update", &fixture).unwrap();
    let mut session = TestSession::new(app, "update");

    // Toggle first item twice (select, then deselect)
    session.toggle_select();
    session.toggle_select();

    insta::with_settings!({
        snapshot_suffix => fixture_name,
        description => session.description()
    }, {
        insta::assert_snapshot!(snapshot(&mut terminal, session.app()));
    });
}

#[rstest]
#[case("root")]
fn test_multi_select_toggle_all(#[case] fixture_name: &str) {
    let fixture = Fixture::load(fixture_name);
    let height = (fixture.inputs.len() as u16 + 3).min(12);
    let mut terminal = create_test_terminal(80, height);
    let app = app_from_args_with_fixture("update", &fixture).unwrap();
    let mut session = TestSession::new(app, "update");

    session.toggle_all();

    insta::with_settings!({
        snapshot_suffix => fixture_name,
        description => session.description()
    }, {
        insta::assert_snapshot!(snapshot(&mut terminal, session.app()));
    });
}

#[rstest]
#[case("root")]
fn test_toggle_diff_in_input(#[case] fixture_name: &str) {
    let fixture = Fixture::load(fixture_name);
    let app = app_from_args_with_fixture("add", &fixture).unwrap();
    let mut session = TestSession::new(app, "add");

    assert!(!session.app().show_diff());
    session.ctrl('d');
    assert!(session.app().show_diff());

    // Use fixed height for input screen (diff preview not visible without content)
    let mut terminal = create_test_terminal(80, 4);
    insta::with_settings!({
        snapshot_suffix => fixture_name,
        description => session.description()
    }, {
        insta::assert_snapshot!(snapshot(&mut terminal, session.app()));
    });
}

#[rstest]
#[case("root")]
fn test_toggle_diff_in_list(#[case] fixture_name: &str) {
    let fixture = Fixture::load(fixture_name);
    let app = app_from_args_with_fixture("pin", &fixture).unwrap();
    let mut session = TestSession::new(app, "pin");

    session.ctrl('d');

    // Use fixed height for list screen
    let height = (fixture.inputs.len() as u16 + 3).min(12);
    let mut terminal = create_test_terminal(80, height);
    insta::with_settings!({
        snapshot_suffix => fixture_name,
        description => session.description()
    }, {
        insta::assert_snapshot!(snapshot(&mut terminal, session.app()));
    });
}

#[rstest]
#[case("root")]
fn test_input_editing_clear(#[case] fixture_name: &str) {
    let fixture = Fixture::load(fixture_name);
    let mut terminal = create_test_terminal(80, 4);
    let app = app_from_args_with_fixture("add github:nixos/nixpkgs", &fixture).unwrap();
    let mut session = TestSession::new(app, "add github:nixos/nixpkgs");

    session.ctrl('u');

    insta::with_settings!({
        snapshot_suffix => fixture_name,
        description => session.description()
    }, {
        insta::assert_snapshot!(snapshot(&mut terminal, session.app()));
    });
}

#[rstest]
#[case("root")]
fn test_input_editing_backspace(#[case] fixture_name: &str) {
    let fixture = Fixture::load(fixture_name);
    let mut terminal = create_test_terminal(80, 4);
    let app = app_from_args_with_fixture("add github:nixos/nixpkgs", &fixture).unwrap();
    let mut session = TestSession::new(app, "add github:nixos/nixpkgs");

    session.backspace();
    session.backspace();
    session.backspace();

    insta::with_settings!({
        snapshot_suffix => fixture_name,
        description => session.description()
    }, {
        insta::assert_snapshot!(snapshot(&mut terminal, session.app()));
    });
}

#[test]
fn test_confirm_screen() {
    let mut terminal = create_test_terminal(80, 10);
    let diff = r#"@@ -1,3 +1,3 @@
 inputs = {
-  nixpkgs.url = "github:nixos/nixpkgs";
+  nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
 };"#;
    let app = App::confirm("Change", diff);
    insta::assert_snapshot!(snapshot(&mut terminal, &app));
}

#[test]
fn test_confirm_apply() {
    let mut app = App::confirm("Test", "@@ -1 +1 @@\n-old\n+new");
    let result = app.update(key(KeyCode::Char('y')));
    assert!(matches!(result, UpdateResult::Done));
}

#[test]
fn test_confirm_exit() {
    let mut app = App::confirm("Test", "@@ -1 +1 @@\n-old\n+new");
    let result = app.update(key(KeyCode::Char('n')));
    assert!(matches!(result, UpdateResult::Cancelled));
}

#[test]
fn test_confirm_back() {
    let mut app = App::confirm("Test", "@@ -1 +1 @@\n-old\n+new");
    let result = app.update(key(KeyCode::Char('b')));
    assert!(matches!(result, UpdateResult::Done));
}

#[rstest]
#[case("root")]
fn test_terminal_height_input(#[case] fixture_name: &str) {
    let fixture = Fixture::load(fixture_name);
    let app = app_from_args_with_fixture("add", &fixture).unwrap();
    assert_eq!(app.terminal_height(), 4);
}

#[rstest]
#[case("root")]
fn test_terminal_height_list(#[case] fixture_name: &str) {
    let fixture = Fixture::load(fixture_name);
    let app = app_from_args_with_fixture("pin", &fixture).unwrap();
    // inputs + 3 (borders + footer)
    let expected = (fixture.inputs.len() as u16 + 3).min(12);
    assert_eq!(app.terminal_height(), expected);
}

#[test]
fn test_terminal_height_confirm() {
    let diff = "@@ -1,3 +1,3 @@\n line1\n line2\n line3";
    let app = App::confirm("Test", diff);
    assert_eq!(app.terminal_height(), 7);
}
