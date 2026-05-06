use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Serialize;

use crate::cli::{CliArgs, Command, ListFormat};
use crate::config::ConfigError;
use crate::edit::{InputMap, sorted_input_ids};
use crate::input::Follows;
use crate::tui;

use super::commands::{self, CommandError};
use super::editor::Editor;
use super::follow;
use super::state::AppState;

/// JSON output for `flake-edit list --format json`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ListOutput {
    pub inputs: BTreeMap<String, InputView>,
    pub follows: Vec<FollowEdge>,
}

/// One entry in [`ListOutput::inputs`].
///
/// `id` and `url` are unquoted (the in-memory invariant). `flake` mirrors the
/// `inputs.<id>.flake = false;` source-form attribute.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InputView {
    pub id: String,
    pub url: String,
    pub flake: bool,
}

/// One edge in [`ListOutput::follows`].
///
/// - `parent` is the top-level input the follows is *declared on*.
/// - `nested` is the nested input being redirected.
/// - `target` is the rendered [`crate::follows::AttrPath`] the nested input
///   is redirected to.
/// - `kind` distinguishes indirect (URL-less, follows another input) from
///   direct (URL-bearing) declarations.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FollowEdge {
    pub parent: String,
    pub nested: String,
    pub target: String,
    pub kind: FollowEdgeKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FollowEdgeKind {
    Indirect,
    Direct,
}

impl From<&InputMap> for ListOutput {
    fn from(inputs: &InputMap) -> Self {
        let mut input_views: BTreeMap<String, InputView> = BTreeMap::new();
        let mut follows: Vec<FollowEdge> = Vec::new();
        for key in sorted_input_ids(inputs) {
            let input = &inputs[key];
            let parent_id = input.id().as_str().to_string();
            input_views.insert(
                key.clone(),
                InputView {
                    id: parent_id.clone(),
                    url: input.url().to_string(),
                    flake: input.flake,
                },
            );
            for f in input.follows() {
                match f {
                    Follows::Indirect { path, target } => {
                        follows.push(FollowEdge {
                            parent: parent_id.clone(),
                            nested: path.to_string(),
                            target: target.as_ref().map(|t| t.to_string()).unwrap_or_default(),
                            kind: FollowEdgeKind::Indirect,
                        });
                    }
                    Follows::Direct(name, child) => {
                        follows.push(FollowEdge {
                            parent: parent_id.clone(),
                            nested: name.clone(),
                            target: child.url().to_string(),
                            kind: FollowEdgeKind::Direct,
                        });
                    }
                }
            }
        }
        ListOutput {
            inputs: input_views,
            follows,
        }
    }
}

mod root;

pub type Result<T> = std::result::Result<T, HandlerError>;

#[derive(Debug, thiserror::Error)]
pub enum HandlerError {
    #[error(transparent)]
    Command(#[from] CommandError),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    FlakeEdit(#[from] crate::error::FlakeEditError),

    #[error(transparent)]
    Config(#[from] ConfigError),

    #[error("Flake not found")]
    FlakeNotFound,

    #[error("--flake and --lock cannot be used with 'follow [PATHS]'")]
    IncompatibleFollowOptions,
}

/// Application entry point.
///
/// Parses CLI arguments, initializes state, and dispatches to command handlers.
pub fn run(args: CliArgs) -> Result<()> {
    // Batch `follow [PATHS...]` runs before creating Editor/AppState because
    // it owns its own per-file Editor/AppState pairs.
    if let Command::Follow {
        paths,
        transitive,
        depth,
    } = args.subcommand()
        && !paths.is_empty()
    {
        if args.flake().is_some() || args.lock_file().is_some() {
            return Err(HandlerError::IncompatibleFollowOptions);
        }
        return follow::auto::run_batch(paths, *transitive, *depth, &args)
            .map_err(HandlerError::Command);
    }

    let flake_path = if let Some(flake) = args.flake() {
        let path = PathBuf::from(flake);
        if path.is_dir() {
            let flake_nix = path.join("flake.nix");
            if !flake_nix.exists() {
                return Err(HandlerError::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("No `flake.nix` found in directory: {}", path.display()),
                )));
            }
            flake_nix
        } else {
            path
        }
    } else {
        let path = PathBuf::from("flake.nix");
        let binding = root::Root::from_path(path).map_err(|_| HandlerError::FlakeNotFound)?;
        binding.path().to_path_buf()
    };

    let editor = Editor::from_path(flake_path.clone())?;
    let mut flake_edit = editor.create_flake_edit()?;
    let interactive = tui::is_interactive(args.non_interactive());

    let no_cache = args.no_cache();
    let mut state = AppState::new(editor.text(), flake_path, args.config().map(PathBuf::from))?
        .with_diff(args.diff())
        .with_no_lock(args.no_lock())
        .with_interactive(interactive)
        .with_lock_file(args.lock_file().map(PathBuf::from))
        .with_no_cache(no_cache)
        .with_cache_path(args.cache().map(PathBuf::from));

    match args.subcommand() {
        Command::Add {
            uri,
            ref_or_rev,
            id,
            no_flake,
            shallow,
        } => {
            commands::add(
                &editor,
                &mut flake_edit,
                &state,
                id.clone(),
                uri.clone(),
                commands::UriOptions {
                    ref_or_rev: ref_or_rev.as_deref(),
                    shallow: *shallow,
                    no_flake: *no_flake,
                },
            )?;
        }

        Command::Remove { id } => {
            commands::remove(&editor, &mut flake_edit, &state, id.clone())?;
        }

        Command::Change {
            uri,
            ref_or_rev,
            id,
            shallow,
        } => {
            commands::change(
                &editor,
                &mut flake_edit,
                &state,
                id.clone(),
                uri.clone(),
                ref_or_rev.as_deref(),
                *shallow,
            )?;
        }

        Command::List { format } => {
            commands::list(&mut flake_edit, format)?;
        }

        Command::Update { id, init } => {
            commands::update(&editor, &mut flake_edit, &state, id.clone(), *init)?;
        }

        Command::Pin { id, rev } => {
            commands::pin(&editor, &mut flake_edit, &state, id.clone(), rev.clone())?;
        }

        Command::Unpin { id } => {
            commands::unpin(&editor, &mut flake_edit, &state, id.clone())?;
        }

        Command::Follow {
            paths: _,
            transitive,
            depth,
        } => {
            // The batch path is handled above. This branch runs on the current flake.
            if let Some(min) = transitive {
                state.config.follow.transitive_min = *min;
            }
            if let Some(max) = depth {
                state.config.follow.max_depth = *max;
            }
            state.lock_offline = true;
            follow::auto::run(&editor, &mut flake_edit, &state)?;
        }

        Command::AddFollow { input, target } => {
            state.lock_offline = true;
            follow::add_follow(
                &editor,
                &mut flake_edit,
                &state,
                input.clone(),
                target.clone(),
            )?;
        }

        Command::Completion { inputs: _, mode } => {
            use crate::cache::{Cache, DEFAULT_URI_TYPES};
            use crate::cli::CompletionMode;
            match mode {
                CompletionMode::Add => {
                    for uri_type in DEFAULT_URI_TYPES {
                        println!("{}", uri_type);
                    }
                    let cache = Cache::load();
                    for uri in cache.list_uris() {
                        println!("{}", uri);
                    }
                    std::process::exit(0);
                }
                CompletionMode::Change => {
                    let inputs = flake_edit.list();
                    crate::cache::populate_cache_from_input_map(inputs, no_cache);
                    for id in inputs.keys() {
                        println!("{}", id);
                    }
                    std::process::exit(0);
                }
                CompletionMode::Follow => {
                    if let Ok(lock) = crate::lock::FlakeLock::from_default_path() {
                        for path in lock.nested_input_paths() {
                            println!("{}", path);
                        }
                    }
                    std::process::exit(0);
                }
                CompletionMode::None => {}
            }
        }

        Command::Config {
            print_default,
            path,
        } => {
            commands::config(*print_default, *path)?;
            return Ok(());
        }
    }

    // Build up the completion cache as users interact with different flakes,
    // not only when they add inputs explicitly.
    crate::cache::populate_cache_from_input_map(flake_edit.curr_list(), no_cache);

    Ok(())
}

/// List inputs in the specified format.
pub fn list_inputs(inputs: &InputMap, format: &ListFormat) {
    match format {
        ListFormat::Simple => list_simple(inputs),
        ListFormat::Json => list_json(inputs),
        ListFormat::Detailed => list_detailed(inputs),
        ListFormat::Raw => list_raw(inputs),
        ListFormat::Toplevel => list_toplevel(inputs),
        ListFormat::None => unreachable!("Should not be possible"),
    }
}

fn list_simple(inputs: &InputMap) {
    let mut buf = String::new();
    for key in sorted_input_ids(inputs) {
        let input = &inputs[key];
        if !buf.is_empty() {
            buf.push('\n');
        }
        buf.push_str(input.id().as_str());
        for follows in input.follows() {
            if let Follows::Indirect { path, .. } = follows {
                let id = format!("{}.{}", input.id().as_str(), path);
                if !buf.is_empty() {
                    buf.push('\n');
                }
                buf.push_str(&id);
            }
        }
    }
    println!("{buf}");
}

fn list_json(inputs: &InputMap) {
    let out: ListOutput = inputs.into();
    println!("{}", serde_json::to_string(&out).unwrap());
}

fn list_toplevel(inputs: &InputMap) {
    let mut buf = String::new();
    for key in sorted_input_ids(inputs) {
        if !buf.is_empty() {
            buf.push('\n');
        }
        buf.push_str(&key.to_string());
    }
    println!("{buf}");
}

fn list_raw(inputs: &InputMap) {
    let sorted: BTreeMap<_, _> = inputs.iter().collect();
    println!("{:#?}", sorted);
}

/// True if `url` is a top-level follows reference (e.g.,
/// `harmonia/treefmt-nix`) rather than a real URL with a `github:` or `git+`
/// protocol prefix.
fn is_toplevel_follows(url: &str) -> bool {
    !url.is_empty() && !url.contains(':') && url.contains('/') && !url.starts_with('/')
}

fn list_detailed(inputs: &InputMap) {
    let mut buf = String::new();
    for key in sorted_input_ids(inputs) {
        let input = &inputs[key];
        if !buf.is_empty() {
            buf.push('\n');
        }
        let line = if is_toplevel_follows(input.url()) {
            format!("· {} <= {}", input.id().as_str(), input.url())
        } else {
            format!("· {} - {}", input.id().as_str(), input.url())
        };
        buf.push_str(&line);
        for follows in input.follows() {
            if let Follows::Indirect { path, target } = follows {
                // Render an empty `follows = ""` as `=> ""` to mirror the
                // source-flake form. Non-empty targets render bare.
                let target_str = match target {
                    Some(t) => t.to_string(),
                    None => "\"\"".to_string(),
                };
                let id = format!("{}{} => {}", " ".repeat(5), path, target_str);
                if !buf.is_empty() {
                    buf.push('\n');
                }
                buf.push_str(&id);
            }
        }
    }
    println!("{buf}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::FlakeEdit;
    use crate::follows::{AttrPath, Segment};
    use crate::input::{Follows, Input, Range};
    use serde_json::json;

    #[test]
    fn list_output_empty_inputs_is_empty_shape() {
        let inputs: InputMap = InputMap::new();
        let out: ListOutput = (&inputs).into();
        let v = serde_json::to_value(&out).unwrap();
        assert_eq!(v, json!({ "inputs": {}, "follows": [] }));
    }

    #[test]
    fn list_output_single_toplevel_no_follows() {
        let mut inputs = InputMap::new();
        let id = Segment::from_unquoted("nixpkgs").unwrap();
        let mut input = Input::new(id);
        input.url = "github:nixos/nixpkgs/nixos-unstable".into();
        inputs.insert("nixpkgs".into(), input);
        let v = serde_json::to_value(ListOutput::from(&inputs)).unwrap();
        assert_eq!(
            v,
            json!({
                "inputs": {
                    "nixpkgs": {
                        "id": "nixpkgs",
                        "url": "github:nixos/nixpkgs/nixos-unstable",
                        "flake": true,
                    }
                },
                "follows": [],
            })
        );
    }

    #[test]
    fn list_output_renders_indirect_follows_as_flat_array() {
        let mut inputs = InputMap::new();
        let crane = Segment::from_unquoted("crane").unwrap();
        let mut input = Input::new(crane);
        input.url = "github:ipetkov/crane".into();
        input.range = Range {
            start: 100,
            end: 120,
        };
        input.follows.push(Follows::Indirect {
            path: AttrPath::new(Segment::from_unquoted("nixpkgs").unwrap()),
            target: Some(AttrPath::parse("nixpkgs").unwrap()),
        });
        inputs.insert("crane".into(), input);
        let v = serde_json::to_value(ListOutput::from(&inputs)).unwrap();
        assert_eq!(
            v,
            json!({
                "inputs": {
                    "crane": {
                        "id": "crane",
                        "url": "github:ipetkov/crane",
                        "flake": true,
                    }
                },
                "follows": [
                    {
                        "parent": "crane",
                        "nested": "nixpkgs",
                        "target": "nixpkgs",
                        "kind": "indirect"
                    }
                ],
            })
        );
    }

    #[test]
    fn list_output_url_is_unquoted() {
        // URLs are stored unquoted in memory. The ListOutput JSON wire form
        // surfaces them unquoted too.
        let mut inputs = InputMap::new();
        let id = Segment::from_unquoted("nixpkgs").unwrap();
        let mut input = Input::new(id);
        input.url = "github:nixos/nixpkgs".into();
        inputs.insert("nixpkgs".into(), input);
        let s = serde_json::to_string(&ListOutput::from(&inputs)).unwrap();
        assert!(!s.contains("\\\"github:"));
        assert!(s.contains("\"url\":\"github:nixos/nixpkgs\""));
    }

    #[test]
    fn list_output_kind_serialises_kebab_case() {
        let edge = FollowEdge {
            parent: "a".into(),
            nested: "b".into(),
            target: "c".into(),
            kind: FollowEdgeKind::Indirect,
        };
        let v = serde_json::to_value(&edge).unwrap();
        assert_eq!(v.get("kind").unwrap(), &json!("indirect"));
    }

    #[test]
    fn list_output_inputs_sorted_by_id() {
        let content = r#"{
            inputs.zzz.url = "github:ex/zzz";
            inputs.aaa.url = "github:ex/aaa";
            outputs = { ... }: { };
        }
        "#;
        let mut fe = FlakeEdit::from_text(content).unwrap();
        let v = serde_json::to_value(ListOutput::from(fe.list())).unwrap();
        let keys: Vec<&str> = v
            .get("inputs")
            .unwrap()
            .as_object()
            .unwrap()
            .keys()
            .map(|s| s.as_str())
            .collect();
        assert_eq!(keys, vec!["aaa", "zzz"]);
    }

    #[test]
    fn test_is_toplevel_follows() {
        for url in [
            "harmonia/treefmt-nix",
            "clan-core/treefmt-nix",
            "clan-core/systems",
        ] {
            assert!(is_toplevel_follows(url), "{url} should be a follows ref");
        }
        for url in [
            "github:NixOS/nixpkgs",
            "git+https://git.clan.lol/clan/clan-core",
            "path:/some/local/path",
            "https://github.com/pinpox.keys",
            "/nix/store/abc",
            "nixpkgs",
            "",
        ] {
            assert!(
                !is_toplevel_follows(url),
                "{url} should not be a follows ref",
            );
        }
    }
}
