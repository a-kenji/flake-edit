//! User-facing error renderer.
//!
//! Cargo-style `error:` / `warning:` / `hint:` prefixes, with a chained
//! `caused by:` block for nested sources and bullet lists for aggregate
//! variants (validation errors and batch failures).
//!
//! `NO_COLOR` is honored: prefix keywords are styled red / yellow / cyan
//! when color is on, plain when it's off. Bodies are never colored.

use std::io::{self, Write as _};

use flake_edit::app;
use flake_edit::app::error::chain_layers;

/// Print an error to stderr in the documented user-facing shape.
pub(crate) fn report(err: &app::Error) {
    let mut stderr = io::stderr().lock();
    let style = Style::detect();

    let _ = write_error_line(&mut stderr, &style, &err.to_string());

    if let Some(bullets) = err.validation_bullets() {
        for line in bullets {
            let _ = writeln!(stderr, "  - {line}");
        }
    }

    // Library validation aggregates land here wrapped as `Error::Flake`;
    // `validation_bullets()` only matches the application-side variant, so
    // pull bullets off the inner library error too.
    if let app::Error::Flake(inner) = err
        && let Some(bullets) = inner.bullets()
    {
        for line in bullets {
            let _ = writeln!(stderr, "  - {line}");
        }
    }

    if let Some(bullets) = err.batch_bullets() {
        for line in bullets {
            let _ = writeln!(stderr, "  - {line}");
        }
    }

    if let Some(bullets) = err.candidate_bullets() {
        for line in bullets {
            let _ = writeln!(stderr, "  - {line}");
        }
    }

    write_caused_by_chain(&mut stderr, &style, err);

    if let Some(hint) = hint_for(err) {
        let _ = writeln!(stderr);
        let _ = write_hint_line(&mut stderr, &style, &hint);
    }
}

/// Style policy. The prefix keywords are colored when `NO_COLOR` is unset.
#[derive(Copy, Clone)]
struct Style {
    color: bool,
}

impl Style {
    fn detect() -> Self {
        Self {
            color: std::env::var("NO_COLOR").is_err(),
        }
    }

    fn error(self) -> &'static str {
        if self.color {
            "\x1b[1;31merror\x1b[0m"
        } else {
            "error"
        }
    }

    fn hint(self) -> &'static str {
        if self.color {
            "\x1b[1;36mhint\x1b[0m"
        } else {
            "hint"
        }
    }

    fn caused_by(self) -> &'static str {
        if self.color {
            "\x1b[2mcaused by\x1b[0m"
        } else {
            "caused by"
        }
    }
}

fn write_error_line(out: &mut impl io::Write, style: &Style, message: &str) -> io::Result<()> {
    writeln!(out, "{}: {}", style.error(), message)
}

fn write_hint_line(out: &mut impl io::Write, style: &Style, message: &str) -> io::Result<()> {
    writeln!(out, "{}: {}", style.hint(), message)
}

/// Render the source chain cargo-style.
///
/// `chain_layers` already collapses adjacent duplicates (`#[error(transparent)]`
/// wrappers) and yields the layers top-down. Skip the first since the
/// `error:` line carries it.
fn write_caused_by_chain(out: &mut impl io::Write, style: &Style, err: &app::Error) {
    for layer in chain_layers(err).into_iter().skip(1) {
        let _ = writeln!(out, "  {}: {}", style.caused_by(), layer);
    }
}

/// Hint string for an `app::Error`, when one applies. Hints are
/// actionable suggestions; they are skipped when redundant with the
/// headline.
fn hint_for(err: &app::Error) -> Option<String> {
    use app::Error;
    use app::error::ToggleAction;
    match err {
        Error::Flake(inner) => inner.hint(),
        Error::FollowsCreateFailed { id } => Some(format!(
            "check that '{id}' is declared in flake.nix; run `flake-edit list` to verify input names; \
             use dot notation `flake-edit follow <input>.<nested-input> <target>` for deeper paths"
        )),
        Error::FlakeNotFound { .. } | Error::FlakeDirEmpty { .. } => Some(
            "run `nix flake init` here, or pass `--flake <path>` pointing at a directory \
             containing flake.nix"
                .into(),
        ),
        Error::LockFile { .. } => Some("run `nix flake lock` to (re)generate flake.lock".into()),
        Error::Batch { .. } => {
            Some("run `flake-edit list` against each failing file to verify input names".into())
        }
        Error::NoToggleableInputs => Some(
            "an input is toggleable when a commented alternate sits next to its url, e.g.\n        \
             rust-overlay.url = \"github:oxalica/rust-overlay\";\n        \
             # rust-overlay.url = \"github:a-kenji/rust-overlay\";\n      \
             store one with `flake-edit toggle <id> <ref>`"
                .into(),
        ),
        Error::MultipleToggleableInputs { .. } => {
            Some("pick one with `flake-edit toggle <id>`".into())
        }
        Error::ToggleUnknownInput { .. } => {
            Some("run `flake-edit list` to see the current inputs".into())
        }
        Error::ToggleNoAlternate { id } => Some(format!(
            "store one and switch to it with `flake-edit toggle {id} <ref>`"
        )),
        Error::ToggleAmbiguousVariant { id, action, .. } => match action {
            ToggleAction::Activate => {
                Some(format!("name the variant: `flake-edit toggle {id} <ref>`"))
            }
            ToggleAction::Remove => Some(format!(
                "name the variant: `flake-edit toggle --remove {id} <ref>`"
            )),
        },
        Error::ToggleRefUnmatched { reference } => Some(format!(
            "no variant, git remote, or directory name corresponds to an input; name it\n      \
             explicitly: `flake-edit toggle <id> {reference}`"
        )),
        Error::ToggleRefAmbiguous { reference, .. } => Some(format!(
            "name the input explicitly: `flake-edit toggle <id> {reference}`"
        )),
        Error::TogglePathMissing { .. } => {
            Some("check the path, or pass a remote ref like `github:owner/repo`".into())
        }
        Error::ToggleAlreadyActive { id, .. } => Some(format!(
            "store one with `flake-edit toggle {id} <other-ref>`"
        )),
        Error::ToggleRemoveUnstored { id, .. } => Some(format!(
            "name one of the stored variants: `flake-edit toggle --remove {id} <ref>`"
        )),
        Error::ToggleRemoveActive { id, .. } => Some(format!(
            "set a new url with `flake-edit change {id} <uri>`, or drop the input with \
             `flake-edit remove {id}`"
        )),
        _ => None,
    }
}
