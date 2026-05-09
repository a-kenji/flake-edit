use nix_uri::{FlakeRef, NixUriError};

use crate::change::Change;

use super::{Error, Result};

/// URI rewriting options that apply to both `add` and `change`.
///
/// `no_flake` is *not* part of this shape: it sets the
/// `inputs.<id>.flake` attribute on the resulting input and is only
/// meaningful for `add`. It travels separately so a caller cannot
/// accidentally request it for `change`, where it would be silently
/// dropped.
#[derive(Default)]
pub struct UriOptions<'a> {
    pub ref_or_rev: Option<&'a str>,
    pub shallow: bool,
}

/// Selects which [`Change`] variant [`build_uri_change`] constructs.
pub(super) enum BuildKind {
    Add { no_flake: bool },
    Change,
}

/// Builds the [`Change::Add`] or [`Change::Change`] requested by the
/// scripted `id + uri` paths in [`super::add`] and [`super::change`].
pub(super) fn build_uri_change(
    kind: BuildKind,
    id: String,
    uri: String,
    opts: &UriOptions<'_>,
) -> Result<Change> {
    let final_uri = transform_uri(uri, opts.ref_or_rev, opts.shallow)?;
    Ok(match kind {
        BuildKind::Add { no_flake } => Change::Add {
            id: Some(id),
            uri: Some(final_uri),
            flake: !no_flake,
        },
        BuildKind::Change => Change::Change {
            id: Some(id),
            uri: Some(final_uri),
        },
    })
}

/// Applies `ref_or_rev` and `shallow` to `flake_ref`, leaving every
/// other field untouched.
///
/// # Errors
///
/// Returns the typed `nix_uri` error when the underlying URI type does
/// not accept the requested option. The renderer attaches an actionable
/// hint at the binary boundary; the library error stays clean.
pub(super) fn apply_uri_options(
    mut flake_ref: FlakeRef,
    ref_or_rev: Option<&str>,
    shallow: bool,
) -> std::result::Result<FlakeRef, NixUriError> {
    if let Some(ror) = ref_or_rev {
        flake_ref.r#type.ref_or_rev(Some(ror.to_string()))?;
    }
    if shallow {
        flake_ref.params.set_shallow(Some("1".to_string()));
    }
    Ok(flake_ref)
}

/// Applies `ref_or_rev` and `shallow` to a URI string, returning the
/// rewritten form.
///
/// The URI is always parsed through `nix-uri` so callers get an
/// early [`Error::InvalidUri`] on malformed input. When neither option
/// is set the original `uri` is returned verbatim to avoid re-rendering
/// query parameters the user typed deliberately.
pub(super) fn transform_uri(
    uri: String,
    ref_or_rev: Option<&str>,
    shallow: bool,
) -> Result<String> {
    let flake_ref: FlakeRef = uri.parse().map_err(|source| Error::InvalidUri {
        uri: uri.clone(),
        source,
    })?;

    if ref_or_rev.is_none() && !shallow {
        return Ok(uri);
    }

    apply_uri_options(flake_ref, ref_or_rev, shallow)
        .map(|f| f.to_string())
        .map_err(|source| Error::ApplyUriOptions {
            uri: uri.clone(),
            source,
        })
}
