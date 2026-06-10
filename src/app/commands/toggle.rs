//! `flake-edit toggle`: flip an input between its active url and a
//! stored alternate.
//!
//! `toggle [INPUT] [REF]`, and any omitted coordinate is inferred. A
//! single positional argument is a ref when it contains `:` or `/` or
//! starts with `.` or `~`, an input id otherwise. A ref resolves to its
//! input through a three-stage matching ladder (variant equality, repo
//! identity, name). The first stage that yields candidates decides.
//! Ambiguity opens the standard fuzzy picker interactively and errors
//! with the candidate list otherwise.
//!
//! `--remove` deletes the resolved variant's line instead of activating
//! it. A commented alternate is dropped in place; the lockfile refresh
//! is skipped because the resolved source cannot change. Naming the
//! active url flips to the stored alternate first and deletes the
//! previously active line, the inverse of first-use synthesis. Removal
//! never synthesizes, and its path-shaped refs may point at deleted
//! directories (matching falls back to the literal spelling), since
//! cleaning up a stale alternate is precisely the removal use case.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use nix_uri::FlakeRef;

use crate::change::{Change, ChangeId};
use crate::edit::{FlakeEdit, ToggleState};
use crate::error::Error as FlakeError;

use super::super::editor::Editor;
use super::super::error::{RefCandidate, ToggleAction, ToggleCandidate};
use super::super::state::AppState;
use super::{ConfirmResult, Error, Result, apply_change, confirm_or_apply, pick_one};

pub fn toggle(
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
    input: Option<String>,
    reference: Option<String>,
    remove: bool,
) -> Result<()> {
    let states = flake_edit.toggle_states()?;
    let flake_dir = flake_dir(state);
    let res = Resolve {
        states: &states,
        flake_dir: &flake_dir,
        action: if remove {
            ToggleAction::Remove
        } else {
            ToggleAction::Activate
        },
    };

    match (input, reference) {
        (None, None) => toggle_no_args(editor, flake_edit, state, &res),
        (Some(arg), None) if is_ref_shaped(&arg) => {
            let ref_arg = RefArg::parse(&arg, res.action)?;
            toggle_by_ref(editor, flake_edit, state, &res, &ref_arg)
        }
        (Some(id), None) => {
            validate_id(flake_edit, res.states, &id)?;
            act_within_input(editor, flake_edit, state, &res, &id)
        }
        (Some(id), Some(reference)) => {
            validate_id(flake_edit, res.states, &id)?;
            let ref_arg = RefArg::parse(&reference, res.action)?;
            act_on_ref(editor, flake_edit, state, &res, &id, &ref_arg)
        }
        (None, Some(_)) => unreachable!("clap fills positional arguments in order"),
    }
}

/// Read-only resolution context shared by every invocation form.
struct Resolve<'a> {
    /// Toggle surface per input id, from [`FlakeEdit::toggle_states`].
    states: &'a BTreeMap<String, ToggleState>,
    /// Directory of the edited `flake.nix`, for relative `path:` variants.
    flake_dir: &'a Path,
    /// What to do with the resolved variant.
    action: ToggleAction,
}

/// Directory holding the edited `flake.nix`. Relative `path:` variants
/// stored in the file resolve against it.
fn flake_dir(state: &AppState) -> PathBuf {
    match state.flake_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// Shape-based classification of the single positional argument.
fn is_ref_shaped(arg: &str) -> bool {
    arg.contains(':') || arg.contains('/') || arg.starts_with('.') || arg.starts_with('~')
}

/// The id must parse, name a declared input, and that input must carry a
/// url binding.
fn validate_id(
    flake_edit: &FlakeEdit,
    states: &BTreeMap<String, ToggleState>,
    id: &str,
) -> Result<()> {
    ChangeId::parse(id).map_err(|source| Error::InvalidInputId {
        id: id.to_string(),
        source,
    })?;
    if !flake_edit.curr_list().contains_key(id) {
        return Err(Error::ToggleUnknownInput { id: id.to_string() });
    }
    if !states.contains_key(id) {
        return Err(FlakeError::NoUrlToToggle(id.to_string()).into());
    }
    Ok(())
}

/// A parsed, normalized ref argument.
struct RefArg {
    /// Exactly what the user typed.
    typed: String,
    /// The form a synthesized alternate stores: the typed text, with only
    /// a `path:` prefix added to bare directories.
    store_as: String,
    /// Canonicalized location for path-shaped refs whose directory
    /// exists. Existence is the typo guard for activation, and the
    /// ladder's later stages need the directory; a removal target may be
    /// gone, leaving only the literal spelling to match on.
    canonical_path: Option<PathBuf>,
    /// Parsed flake reference for everything else.
    flake_ref: Option<FlakeRef>,
}

impl RefArg {
    fn parse(typed: &str, action: ToggleAction) -> Result<Self> {
        if let Some(path) = path_part(typed) {
            let canonical_path = match std::fs::canonicalize(path) {
                Ok(canonical) => Some(canonical),
                // A removal may name an alternate whose directory is
                // gone; matching falls back to the literal spelling.
                Err(_) if action == ToggleAction::Remove => None,
                Err(source) => {
                    return Err(Error::TogglePathMissing {
                        path: typed.to_string(),
                        source,
                    });
                }
            };
            let store_as = if typed.starts_with("path:") {
                typed.to_string()
            } else {
                format!("path:{typed}")
            };
            Ok(Self {
                typed: typed.to_string(),
                store_as,
                canonical_path,
                flake_ref: None,
            })
        } else {
            let flake_ref: FlakeRef = typed.parse().map_err(|source| Error::InvalidUri {
                uri: typed.to_string(),
                source,
            })?;
            Ok(Self {
                typed: typed.to_string(),
                store_as: typed.to_string(),
                canonical_path: None,
                flake_ref: Some(flake_ref),
            })
        }
    }
}

/// Filesystem part of a path-shaped ref, or `None` for non-path refs.
fn path_part(typed: &str) -> Option<&str> {
    if let Some(rest) = typed.strip_prefix("path:") {
        return Some(rest);
    }
    (typed.starts_with('.') || typed.starts_with('/') || typed.starts_with('~')).then_some(typed)
}

fn variants(state: &ToggleState) -> impl Iterator<Item = &String> {
    std::iter::once(&state.active).chain(state.alternates.iter())
}

/// Stage-1 equality: the ref equals `variant` after normalization
/// (canonicalized paths for `path:` shapes, canonical `nix_uri` strings
/// otherwise). A dead path, possible only as a removal target, matches
/// by its literal spelling.
fn variant_equals(variant: &str, ref_arg: &RefArg, flake_dir: &Path) -> bool {
    if let Some(canonical) = &ref_arg.canonical_path {
        return variant_path(variant, flake_dir).is_some_and(|p| p == *canonical);
    }
    if let Some(flake_ref) = &ref_arg.flake_ref {
        return variant
            .parse::<FlakeRef>()
            .is_ok_and(|v| v.to_canonical_string() == flake_ref.to_canonical_string());
    }
    variant == ref_arg.store_as || variant == ref_arg.typed
}

/// Canonical filesystem location of a path-style variant. Relative paths
/// stored in `flake.nix` are relative to the flake's directory.
fn variant_path(variant: &str, flake_dir: &Path) -> Option<PathBuf> {
    let raw = variant
        .strip_prefix("path:")
        .or_else(|| (variant.starts_with('/') || variant.starts_with('.')).then_some(variant))?;
    let absolute = if Path::new(raw).is_absolute() {
        PathBuf::from(raw)
    } else {
        flake_dir.join(raw)
    };
    std::fs::canonicalize(absolute).ok()
}

/// `(host, owner, repo)` identity used by the ladder's second stage,
/// lowercased for comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RepoIdentity {
    host: String,
    owner: String,
    repo: String,
}

impl RepoIdentity {
    fn new(host: &str, owner: &str, repo: &str) -> Self {
        Self {
            host: host.to_lowercase(),
            owner: owner.to_lowercase(),
            repo: repo.trim_end_matches(".git").to_lowercase(),
        }
    }
}

/// Identity granularities, tried loosest-last. The looser ones let a fork
/// checkout whose `origin` is the fork find the input declared with
/// upstream's owner.
#[derive(Clone, Copy)]
enum Granularity {
    HostOwnerRepo,
    HostRepo,
    Repo,
}

impl Granularity {
    fn matches(self, a: &RepoIdentity, b: &RepoIdentity) -> bool {
        match self {
            Self::HostOwnerRepo => a == b,
            Self::HostRepo => a.host == b.host && a.repo == b.repo,
            Self::Repo => a.repo == b.repo,
        }
    }
}

/// Identities the ref itself resolves to: the parsed forge coordinates,
/// or every configured git remote of a path checkout. A path that is not
/// a git checkout contributes nothing here.
fn ref_identities(ref_arg: &RefArg) -> Vec<RepoIdentity> {
    if let Some(path) = &ref_arg.canonical_path {
        return git_remote_urls(path)
            .iter()
            .filter_map(|url| parse_remote_url(url))
            .collect();
    }
    ref_arg
        .flake_ref
        .as_ref()
        .and_then(flake_ref_identity)
        .into_iter()
        .collect()
}

fn flake_ref_identity(flake_ref: &FlakeRef) -> Option<RepoIdentity> {
    if let Some(forge) = flake_ref.forge_identity() {
        return Some(RepoIdentity::new(&forge.domain, &forge.owner, &forge.repo));
    }
    match (flake_ref.domain(), flake_ref.owner(), flake_ref.repo()) {
        (Some(domain), Some(owner), Some(repo)) => Some(RepoIdentity::new(domain, owner, repo)),
        _ => None,
    }
}

fn variant_identity(variant: &str) -> Option<RepoIdentity> {
    flake_ref_identity(&variant.parse::<FlakeRef>().ok()?)
}

/// Remote URLs from the checkout's git config. Reading the config keeps
/// the heuristic local and fast. Failures (no git, not a repo) degrade to
/// an empty list.
fn git_remote_urls(dir: &Path) -> Vec<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["config", "--get-regexp", r"^remote\..*\.url$"])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.split_once(' ').map(|(_, url)| url.trim().to_string()))
        .collect()
}

/// Parse a git remote URL to its identity. Handles scp-like syntax
/// (`git@host:owner/repo.git`) and URL forms. The owner is the path
/// segment directly before the repo.
fn parse_remote_url(url: &str) -> Option<RepoIdentity> {
    let (host, path) = if let Some((_, rest)) = url.split_once("://") {
        let (authority, path) = rest.split_once('/')?;
        let host = authority.rsplit('@').next()?.split(':').next()?;
        (host, path)
    } else if let Some((authority, path)) = url.split_once(':') {
        (authority.rsplit('@').next()?, path)
    } else {
        return None;
    };
    let path = path.trim_end_matches('/').trim_end_matches(".git");
    let mut segments = path.rsplit('/');
    let repo = segments.next()?;
    if repo.is_empty() {
        return None;
    }
    let owner = segments.next().unwrap_or("");
    Some(RepoIdentity::new(host, owner, repo))
}

/// Repo or directory name the ref answers to, for the ladder's third
/// stage.
fn ref_name(ref_arg: &RefArg) -> Option<String> {
    if let Some(path) = &ref_arg.canonical_path {
        return path.file_name().map(|n| n.to_string_lossy().into_owned());
    }
    if let Some(flake_ref) = &ref_arg.flake_ref {
        return flake_ref
            .repo()
            .or_else(|| flake_ref.id())
            .map(|name| name.trim_end_matches(".git").to_string());
    }
    // A dead path keeps the basename of its typed spelling.
    Path::new(path_part(&ref_arg.typed)?)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
}

/// The matching ladder of the ref form. The first stage that yields any
/// candidates decides. Later stages are not consulted.
fn ladder(
    states: &BTreeMap<String, ToggleState>,
    ref_arg: &RefArg,
    flake_dir: &Path,
) -> Vec<RefCandidate> {
    // Stage 1: variant equality.
    let mut found: Vec<RefCandidate> = states
        .iter()
        .filter_map(|(id, state)| {
            variants(state)
                .find(|v| variant_equals(v, ref_arg, flake_dir))
                .map(|variant| RefCandidate {
                    id: id.clone(),
                    reason: format!("{variant} is a stored variant"),
                })
        })
        .collect();
    if !found.is_empty() {
        return found;
    }

    // Stage 2: repo identity, three granularities, first hit decides.
    let ref_ids = ref_identities(ref_arg);
    if !ref_ids.is_empty() {
        let how = if ref_arg.canonical_path.is_some() {
            "matches a git remote"
        } else {
            "matches the repository"
        };
        for granularity in [
            Granularity::HostOwnerRepo,
            Granularity::HostRepo,
            Granularity::Repo,
        ] {
            let hits: Vec<RefCandidate> = states
                .iter()
                .filter_map(|(id, state)| {
                    variants(state)
                        .find(|v| {
                            variant_identity(v).is_some_and(|vid| {
                                ref_ids.iter().any(|rid| granularity.matches(rid, &vid))
                            })
                        })
                        .map(|variant| RefCandidate {
                            id: id.clone(),
                            reason: format!("{variant} {how}"),
                        })
                })
                .collect();
            if !hits.is_empty() {
                return hits;
            }
        }
    }

    // Stage 3: the directory basename or repo name equals an input id.
    if let Some(name) = ref_name(ref_arg) {
        found = states
            .keys()
            .filter(|id| **id == name)
            .map(|id| RefCandidate {
                id: id.clone(),
                reason: "the name matches the input id".to_string(),
            })
            .collect();
    }
    found
}

/// What activating a ref means for one resolved input.
enum RefTarget {
    /// The ref equals a stored alternate, so uncomment it.
    Activate(String),
    /// The ref equals the active variant, so flip back to the stored side.
    ActiveNamed,
    /// Not stored yet, so synthesize a new alternate.
    Synthesize(String),
}

fn target_for_ref(state: &ToggleState, ref_arg: &RefArg, flake_dir: &Path) -> RefTarget {
    if let Some(alternate) = state
        .alternates
        .iter()
        .find(|v| variant_equals(v, ref_arg, flake_dir))
    {
        return RefTarget::Activate(alternate.clone());
    }
    if variant_equals(&state.active, ref_arg, flake_dir) {
        return RefTarget::ActiveNamed;
    }
    RefTarget::Synthesize(ref_arg.store_as.clone())
}

fn toggle_change(id: &str, state: &ToggleState, uri: String) -> Result<Change> {
    let change_id = ChangeId::parse(id).map_err(|source| Error::InvalidInputId {
        id: id.to_string(),
        source,
    })?;
    Ok(Change::Toggle {
        id: change_id,
        uri,
        previous: state.active.clone(),
    })
}

fn remove_change(id: &str, uri: String, activate: Option<String>) -> Result<Change> {
    let change_id = ChangeId::parse(id).map_err(|source| Error::InvalidInputId {
        id: id.to_string(),
        source,
    })?;
    Ok(Change::ToggleRemove {
        id: change_id,
        uri,
        activate,
    })
}

/// The change acting on one inactive variant of `id`: activate it, or
/// delete its comment line.
fn within_change(
    action: ToggleAction,
    id: &str,
    toggle_state: &ToggleState,
    uri: String,
) -> Result<Change> {
    match action {
        ToggleAction::Activate => toggle_change(id, toggle_state, uri),
        ToggleAction::Remove => remove_change(id, uri, None),
    }
}

/// Deleting only a comment line cannot change the resolved source, so
/// such removals skip the lockfile refresh.
fn effective_state(state: &AppState, change: &Change) -> AppState {
    let mut state = state.clone();
    if matches!(change, Change::ToggleRemove { activate: None, .. }) {
        state.no_lock = true;
    }
    state
}

fn apply_toggle_change(
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
    change: Change,
) -> Result<()> {
    let state = effective_state(state, &change);
    apply_change(editor, flake_edit, &state, change)
}

fn toggle_no_args(
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
    res: &Resolve<'_>,
) -> Result<()> {
    let toggleable: Vec<(&String, &ToggleState)> = res
        .states
        .iter()
        .filter(|(_, s)| !s.alternates.is_empty())
        .collect();
    match toggleable.as_slice() {
        [] => Err(Error::NoToggleableInputs),
        [(id, _)] => act_within_input(editor, flake_edit, state, res, id),
        _ if state.interactive => pick_input_and_act(editor, flake_edit, state, res, {
            toggleable
                .iter()
                .map(|(id, s)| ((*id).clone(), format!("{id} ({})", s.active)))
                .collect()
        }),
        _ => Err(Error::MultipleToggleableInputs {
            candidates: toggleable
                .into_iter()
                .map(|(id, s)| ToggleCandidate {
                    id: id.clone(),
                    variants: variants(s).cloned().collect(),
                })
                .collect(),
        }),
    }
}

fn toggle_by_ref(
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
    res: &Resolve<'_>,
    ref_arg: &RefArg,
) -> Result<()> {
    let candidates = ladder(res.states, ref_arg, res.flake_dir);
    match candidates.as_slice() {
        [] => Err(Error::ToggleRefUnmatched {
            reference: ref_arg.typed.clone(),
        }),
        [candidate] => act_on_ref(
            editor,
            flake_edit,
            state,
            res,
            &candidate.id.clone(),
            ref_arg,
        ),
        _ if state.interactive => {
            let items = candidates
                .iter()
                .map(|c| (c.id.clone(), format!("{} ({})", c.id, c.reason)))
                .collect();
            pick_input_and_act_ref(editor, flake_edit, state, res, items, ref_arg)
        }
        _ => Err(Error::ToggleRefAmbiguous {
            reference: ref_arg.typed.clone(),
            candidates,
        }),
    }
}

/// Act on `ref_arg` against the already-resolved input `id`.
///
/// Activation synthesizes a new alternate when the ref is not stored
/// yet; removal refuses to invent one. Naming the active variant flips
/// to the stored side (and, for removal, deletes the previously active
/// line instead of keeping it as a comment).
fn act_on_ref(
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
    res: &Resolve<'_>,
    id: &str,
    ref_arg: &RefArg,
) -> Result<()> {
    let toggle_state = &res.states[id];
    match (
        target_for_ref(toggle_state, ref_arg, res.flake_dir),
        res.action,
    ) {
        (RefTarget::Activate(uri) | RefTarget::Synthesize(uri), ToggleAction::Activate) => {
            let change = toggle_change(id, toggle_state, uri)?;
            apply_toggle_change(editor, flake_edit, state, change)
        }
        (RefTarget::Activate(uri), ToggleAction::Remove) => {
            let change = remove_change(id, uri, None)?;
            apply_toggle_change(editor, flake_edit, state, change)
        }
        (RefTarget::Synthesize(_), ToggleAction::Remove) => Err(Error::ToggleRemoveUnstored {
            reference: ref_arg.typed.clone(),
            id: id.to_string(),
            active: toggle_state.active.clone(),
            alternates: toggle_state.alternates.clone(),
        }),
        (RefTarget::ActiveNamed, ToggleAction::Activate) => {
            match toggle_state.alternates.as_slice() {
                [] => Err(Error::ToggleAlreadyActive {
                    reference: ref_arg.typed.clone(),
                    id: id.to_string(),
                }),
                _ => act_within_input(editor, flake_edit, state, res, id),
            }
        }
        (RefTarget::ActiveNamed, ToggleAction::Remove) => {
            match toggle_state.alternates.as_slice() {
                [] => Err(Error::ToggleRemoveActive {
                    reference: ref_arg.typed.clone(),
                    id: id.to_string(),
                }),
                [only] => {
                    let change =
                        remove_change(id, toggle_state.active.clone(), Some(only.clone()))?;
                    apply_toggle_change(editor, flake_edit, state, change)
                }
                _ if state.interactive => {
                    pick_replacement_and_remove(editor, flake_edit, state, id, toggle_state)
                }
                // The open question is which alternate replaces the
                // removed url, so the hint points at activating one
                // explicitly first.
                _ => Err(Error::ToggleAmbiguousVariant {
                    id: id.to_string(),
                    active: toggle_state.active.clone(),
                    alternates: toggle_state.alternates.clone(),
                    action: ToggleAction::Activate,
                }),
            }
        }
    }
}

/// Act on `id`'s stored side: one alternate flips (or is deleted)
/// silently, several open the variant picker (or error
/// non-interactively), none is an error.
fn act_within_input(
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
    res: &Resolve<'_>,
    id: &str,
) -> Result<()> {
    let toggle_state = &res.states[id];
    match toggle_state.alternates.as_slice() {
        [] => Err(Error::ToggleNoAlternate { id: id.to_string() }),
        [only] => {
            let change = within_change(res.action, id, toggle_state, only.clone())?;
            apply_toggle_change(editor, flake_edit, state, change)
        }
        _ if state.interactive => {
            pick_variant_and_act(editor, flake_edit, state, res.action, id, toggle_state)
        }
        _ => Err(Error::ToggleAmbiguousVariant {
            id: id.to_string(),
            active: toggle_state.active.clone(),
            alternates: toggle_state.alternates.clone(),
            action: res.action,
        }),
    }
}

/// Apply `change` through the confirm-with-diff screen, printing the
/// success line once applied.
fn confirm_toggle(
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
    change: Change,
    show_diff: bool,
) -> Result<Option<bool>> {
    let state = effective_state(state, &change);
    let outcome = flake_edit.apply_change(change.clone())?;
    let Some(text) = outcome.text else {
        println!("Nothing changed.");
        return Ok(Some(true));
    };
    match confirm_or_apply(editor, &state, "Toggle", &text, show_diff)? {
        ConfirmResult::Applied => {
            for msg in change.success_messages() {
                println!("{msg}");
            }
            Ok(Some(true))
        }
        ConfirmResult::Back => Ok(Some(false)),
        ConfirmResult::Cancelled => Ok(None),
    }
}

/// Variant picker for one input: fuzzy-select among the inactive
/// variants, with the active one shown in the prompt for context.
fn pick_variant_and_act(
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
    action: ToggleAction,
    id: &str,
    toggle_state: &ToggleState,
) -> Result<()> {
    let prompt = match action {
        ToggleAction::Activate => format!("Select variant (active: {})", toggle_state.active),
        ToggleAction::Remove => {
            format!("Select variant to remove (active: {})", toggle_state.active)
        }
    };
    loop {
        let Some((uri, show_diff)) =
            pick_one(state, "Toggle", &prompt, toggle_state.alternates.clone())?
        else {
            return Ok(());
        };
        let change = within_change(action, id, toggle_state, uri)?;
        match confirm_toggle(editor, flake_edit, state, change, show_diff)? {
            Some(true) | None => return Ok(()),
            Some(false) => continue,
        }
    }
}

/// Replacement picker for removing the active url: fuzzy-select the
/// alternate that takes its place.
fn pick_replacement_and_remove(
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
    id: &str,
    toggle_state: &ToggleState,
) -> Result<()> {
    let prompt = format!("Select replacement (removing: {})", toggle_state.active);
    loop {
        let Some((uri, show_diff)) =
            pick_one(state, "Toggle", &prompt, toggle_state.alternates.clone())?
        else {
            return Ok(());
        };
        let change = remove_change(id, toggle_state.active.clone(), Some(uri))?;
        match confirm_toggle(editor, flake_edit, state, change, show_diff)? {
            Some(true) | None => return Ok(()),
            Some(false) => continue,
        }
    }
}

/// Input picker for the no-argument form: fuzzy-select among the
/// toggleable inputs, then continue per the variant rules.
fn pick_input_and_act(
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
    res: &Resolve<'_>,
    items: Vec<(String, String)>,
) -> Result<()> {
    let by_item: BTreeMap<String, String> = items
        .iter()
        .map(|(id, item)| (item.clone(), id.clone()))
        .collect();
    loop {
        let Some((item, show_diff)) = pick_one(
            state,
            "Toggle",
            "Select input",
            items.iter().map(|(_, item)| item.clone()).collect(),
        )?
        else {
            return Ok(());
        };
        let id = &by_item[&item];
        let toggle_state = &res.states[id];
        match toggle_state.alternates.as_slice() {
            [only] => {
                let change = within_change(res.action, id, toggle_state, only.clone())?;
                match confirm_toggle(editor, flake_edit, state, change, show_diff)? {
                    Some(true) | None => return Ok(()),
                    Some(false) => continue,
                }
            }
            _ => {
                return pick_variant_and_act(
                    editor,
                    flake_edit,
                    state,
                    res.action,
                    id,
                    toggle_state,
                );
            }
        }
    }
}

/// Input picker for an ambiguous ref: fuzzy-select among the ladder's
/// candidates, then act on the ref against the chosen input.
fn pick_input_and_act_ref(
    editor: &Editor,
    flake_edit: &mut FlakeEdit,
    state: &AppState,
    res: &Resolve<'_>,
    items: Vec<(String, String)>,
    ref_arg: &RefArg,
) -> Result<()> {
    let by_item: BTreeMap<String, String> = items
        .iter()
        .map(|(id, item)| (item.clone(), id.clone()))
        .collect();
    let Some((item, _show_diff)) = pick_one(
        state,
        "Toggle",
        "Select input",
        items.iter().map(|(_, item)| item.clone()).collect(),
    )?
    else {
        return Ok(());
    };
    let id = by_item[&item].clone();
    act_on_ref(editor, flake_edit, state, res, &id, ref_arg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ref_shape_classification() {
        for ref_like in [
            "github:owner/repo",
            "../rust-overlay",
            "./checkout",
            "~/dev/overlay",
            "/abs/path",
            "owner/repo",
            "path:../x",
        ] {
            assert!(is_ref_shaped(ref_like), "{ref_like} should be a ref");
        }
        for id_like in ["rust-overlay", "nixpkgs", "crane"] {
            assert!(!is_ref_shaped(id_like), "{id_like} should be an id");
        }
    }

    #[test]
    fn remote_url_parsing_covers_common_forms() {
        let cases = [
            (
                "https://github.com/oxalica/rust-overlay.git",
                ("github.com", "oxalica", "rust-overlay"),
            ),
            (
                "git@github.com:oxalica/rust-overlay.git",
                ("github.com", "oxalica", "rust-overlay"),
            ),
            (
                "ssh://git@github.com/oxalica/rust-overlay",
                ("github.com", "oxalica", "rust-overlay"),
            ),
            (
                "https://gitlab.com/group/subgroup/repo",
                ("gitlab.com", "subgroup", "repo"),
            ),
        ];
        for (url, (host, owner, repo)) in cases {
            let identity = parse_remote_url(url).expect(url);
            assert_eq!(identity, RepoIdentity::new(host, owner, repo), "{url}");
        }
        assert!(parse_remote_url("not-a-url").is_none());
    }

    #[test]
    fn forge_ref_identity_resolves_canonical_host() {
        let identity = variant_identity("github:NixOS/nixpkgs/nixos-unstable").unwrap();
        assert_eq!(
            identity,
            RepoIdentity::new("github.com", "nixos", "nixpkgs")
        );
    }

    #[test]
    fn granularities_loosen_in_order() {
        let declared = RepoIdentity::new("github.com", "oxalica", "rust-overlay");
        let fork = RepoIdentity::new("github.com", "a-kenji", "rust-overlay");
        let elsewhere = RepoIdentity::new("git.example.org", "mirror", "rust-overlay");
        assert!(!Granularity::HostOwnerRepo.matches(&fork, &declared));
        assert!(Granularity::HostRepo.matches(&fork, &declared));
        assert!(!Granularity::HostRepo.matches(&elsewhere, &declared));
        assert!(Granularity::Repo.matches(&elsewhere, &declared));
    }
}
