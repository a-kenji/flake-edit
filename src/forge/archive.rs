//! Parsing for tarball-archive flake input URLs.
//!
//! Inputs such as
//!
//! ```text
//! inputs.example.url = "https://forge.example.com/owner/repo/archive/25.11.tar.gz";
//! ```
//!
//! point at a forge's *download* endpoint rather than its git remote.

/// The path prefix that sits between `/archive/` and the ref token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RefPrefix {
    /// `/archive/<ref>.<ext>` with no `refs/...` segment.
    None,
    /// `/archive/refs/tags/<ref>.<ext>`.
    Tags,
    /// `/archive/refs/heads/<ref>.<ext>`.
    Heads,
}

impl RefPrefix {
    /// The literal path segment, ready to insert back into a URL.
    ///
    /// Empty for [`RefPrefix::None`] so the same `format!` rebuilds
    /// every variant without a branch.
    fn as_str(self) -> &'static str {
        match self {
            RefPrefix::None => "",
            RefPrefix::Tags => "refs/tags/",
            RefPrefix::Heads => "refs/heads/",
        }
    }
}

/// Archive extensions we recognise. No entry is a suffix of another,
/// so match order does not matter.
const ARCHIVE_EXTENSIONS: [&str; 4] = [".tar.gz", ".tar.xz", ".tar.bz2", ".zip"];

/// A parsed tarball-archive URL, split into the parts the update
/// engine needs to resolve a newer ref and rewrite the URL in place.
///
/// Every field is stored verbatim from the source so
/// [`ArchiveUrl::with_ref`] can reconstruct a byte-identical URL with
/// only the ref token changed. Construct one through
/// [`ArchiveUrl::parse`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArchiveUrl {
    /// `http` or `https`, preserved so an `http`-only forge is not
    /// silently upgraded to `https`.
    scheme: String,
    /// Host including any `:port`, e.g. `github.com`,
    /// `forge.example.com`, or `forge:3000`. Passed to the forge client
    /// as the domain.
    authority: String,
    owner: String,
    repo: String,
    /// `refs/tags/` / `refs/heads/` / none, as it appeared after
    /// `/archive/`.
    ref_prefix: RefPrefix,
    /// The bare ref token with the `refs/...` prefix and the archive
    /// extension stripped, e.g. `25.11`, `v1.0.0`, `nixos-24.05`.
    ref_token: String,
    /// The matched archive extension, leading dot included, e.g.
    /// `.tar.gz`.
    ext: String,
    /// Trailing `?query` (including the leading `?`), re-appended
    /// unchanged by [`ArchiveUrl::with_ref`]; empty when the URL had
    /// none. Lets a pin like `.../archive/25.11.tar.gz?shallow=1` keep
    /// its fetch options across a ref bump.
    tail: String,
}

impl ArchiveUrl {
    /// Parse `uri` as a tarball-archive URL, or return `None` when it
    /// is not one we can act on.
    ///
    /// Accepts the following URL forms:
    /// - Gitea/Forgejo: `<scheme>://<host>/<owner>/<repo>/archive/<ref>.<ext>`
    /// - GitHub: the bare form above plus
    ///   `.../archive/refs/{tags,heads}/<ref>.<ext>`
    ///
    /// with `<scheme>` either `http` or `https`, `<host>` carrying any
    /// `:port`, and `<ext>` one of [`ARCHIVE_EXTENSIONS`].
    pub(crate) fn parse(uri: &str) -> Option<ArchiveUrl> {
        let (scheme, rest) = uri.split_once("://")?;
        if scheme != "http" && scheme != "https" {
            return None;
        }

        let (authority, path_and_tail) = rest.split_once('/')?;
        if authority.is_empty() {
            return None;
        }

        // Split off any `?query` before touching the path, so a `/`
        // inside the query can't be mistaken for a path separator.
        let (path, tail) = match path_and_tail.find('?') {
            Some(idx) => (&path_and_tail[..idx], &path_and_tail[idx..]),
            None => (path_and_tail, ""),
        };

        // owner / repo / "archive" / <ref-path...>
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() < 4 || parts[2] != "archive" {
            return None;
        }
        let owner = parts[0];
        let repo = parts[1];
        if owner.is_empty() || repo.is_empty() {
            return None;
        }

        let rest_segments = &parts[3..];
        let (ref_prefix, ref_segments): (RefPrefix, &[&str]) =
            if rest_segments.len() >= 2 && rest_segments[0] == "refs" && rest_segments[1] == "tags"
            {
                (RefPrefix::Tags, &rest_segments[2..])
            } else if rest_segments.len() >= 2
                && rest_segments[0] == "refs"
                && rest_segments[1] == "heads"
            {
                (RefPrefix::Heads, &rest_segments[2..])
            } else {
                (RefPrefix::None, rest_segments)
            };

        // After the optional `refs/...` prefix the ref must be a single
        // `<ref>.<ext>` segment.
        if ref_segments.len() != 1 {
            return None;
        }
        let ref_with_ext = ref_segments[0];

        let ext = ARCHIVE_EXTENSIONS
            .into_iter()
            .find(|candidate| ref_with_ext.ends_with(*candidate))?;
        let ref_token = &ref_with_ext[..ref_with_ext.len() - ext.len()];
        if ref_token.is_empty() {
            return None;
        }

        Some(ArchiveUrl {
            scheme: scheme.to_string(),
            authority: authority.to_string(),
            owner: owner.to_string(),
            repo: repo.to_string(),
            ref_prefix,
            ref_token: ref_token.to_string(),
            ext: ext.to_string(),
            tail: tail.to_string(),
        })
    }

    /// Rebuild the URL with `new_ref` swapped in for the ref token,
    /// preserving scheme, host, owner, repo, the `refs/...` prefix, the
    /// extension, and any `?query` tail.
    ///
    /// `new_ref` is the bare resolved ref (e.g. `26.05`, `v2.0.0`); the
    /// stored [`RefPrefix`] is re-applied here so callers never have to
    /// reassemble `refs/tags/...` themselves.
    pub(crate) fn with_ref(&self, new_ref: &str) -> String {
        format!(
            "{}://{}/{}/{}/archive/{}{}{}{}",
            self.scheme,
            self.authority,
            self.owner,
            self.repo,
            self.ref_prefix.as_str(),
            new_ref,
            self.ext,
            self.tail,
        )
    }

    pub(crate) fn owner(&self) -> &str {
        &self.owner
    }

    pub(crate) fn repo(&self) -> &str {
        &self.repo
    }

    /// Host including any `:port`.
    pub(crate) fn host(&self) -> &str {
        &self.authority
    }

    /// The bare ref token, with the `refs/...` prefix and extension stripped.
    pub(crate) fn ref_token(&self) -> &str {
        &self.ref_token
    }

    /// The `refs/...` path prefix as a literal string (`""` when absent).
    pub(crate) fn ref_prefix_str(&self) -> &str {
        self.ref_prefix.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gitea_bare_date_channel() {
        let url = ArchiveUrl::parse("https://forge.example.com/owner/repo/archive/25.11.tar.gz")
            .expect("bare date-channel archive URL should parse");
        assert_eq!(url.scheme, "https");
        assert_eq!(url.authority, "forge.example.com");
        assert_eq!(url.owner, "owner");
        assert_eq!(url.repo, "repo");
        assert_eq!(url.ref_prefix, RefPrefix::None);
        assert_eq!(url.ref_token, "25.11");
        assert_eq!(url.ext, ".tar.gz");
        assert_eq!(url.tail, "");
    }

    #[test]
    fn github_bare_semver() {
        let url = ArchiveUrl::parse("https://github.com/owner/repo/archive/v1.0.0.tar.gz")
            .expect("github bare archive URL should parse");
        assert_eq!(url.authority, "github.com");
        assert_eq!(url.owner, "owner");
        assert_eq!(url.repo, "repo");
        assert_eq!(url.ref_prefix, RefPrefix::None);
        assert_eq!(url.ref_token, "v1.0.0");
        assert_eq!(url.ext, ".tar.gz");
    }

    #[test]
    fn github_refs_tags_prefix() {
        let url =
            ArchiveUrl::parse("https://github.com/owner/repo/archive/refs/tags/v1.0.0.tar.gz")
                .expect("github refs/tags archive URL should parse");
        assert_eq!(url.ref_prefix, RefPrefix::Tags);
        assert_eq!(url.ref_token, "v1.0.0");
        assert_eq!(url.ext, ".tar.gz");
    }

    #[test]
    fn github_refs_heads_prefix() {
        let url = ArchiveUrl::parse(
            "https://github.com/owner/repo/archive/refs/heads/nixos-24.05.tar.gz",
        )
        .expect("github refs/heads archive URL should parse");
        assert_eq!(url.ref_prefix, RefPrefix::Heads);
        assert_eq!(url.ref_token, "nixos-24.05");
        assert_eq!(url.ext, ".tar.gz");
    }

    #[test]
    fn http_scheme_and_port_preserved() {
        let url = ArchiveUrl::parse("http://forge:3000/test/project1/archive/v1.0.0.tar.gz")
            .expect("http forge archive URL should parse");
        assert_eq!(url.scheme, "http");
        assert_eq!(url.authority, "forge:3000");
        assert_eq!(url.host(), "forge:3000");
        assert_eq!(url.owner, "test");
        assert_eq!(url.repo, "project1");
        assert_eq!(url.ref_token, "v1.0.0");
    }

    #[test]
    fn every_archive_extension_is_recognised() {
        for ext in [".tar.gz", ".tar.xz", ".tar.bz2", ".zip"] {
            let uri = format!("https://github.com/owner/repo/archive/v1.0.0{ext}");
            let url = ArchiveUrl::parse(&uri)
                .unwrap_or_else(|| panic!("extension {ext} should be recognised"));
            assert_eq!(url.ext, ext);
            assert_eq!(url.ref_token, "v1.0.0");
        }
    }

    #[test]
    fn with_ref_swaps_only_the_ref_token() {
        let url =
            ArchiveUrl::parse("https://forge.example.com/owner/repo/archive/25.11.tar.gz").unwrap();
        assert_eq!(
            url.with_ref("26.05"),
            "https://forge.example.com/owner/repo/archive/26.05.tar.gz"
        );
    }

    #[test]
    fn with_ref_reapplies_refs_tags_prefix() {
        let url =
            ArchiveUrl::parse("https://github.com/owner/repo/archive/refs/tags/v1.0.0.tar.gz")
                .unwrap();
        assert_eq!(
            url.with_ref("v2.0.0"),
            "https://github.com/owner/repo/archive/refs/tags/v2.0.0.tar.gz"
        );
    }

    #[test]
    fn with_ref_reapplies_refs_heads_prefix_and_extension() {
        let url =
            ArchiveUrl::parse("http://forge:3000/nixos/nixpkgs/archive/refs/heads/nixos-24.05.zip")
                .unwrap();
        assert_eq!(
            url.with_ref("nixos-24.11"),
            "http://forge:3000/nixos/nixpkgs/archive/refs/heads/nixos-24.11.zip"
        );
    }

    #[test]
    fn parse_then_with_ref_is_identity_for_same_ref() {
        let uri = "https://github.com/owner/repo/archive/refs/heads/nixos-24.05.tar.xz";
        let url = ArchiveUrl::parse(uri).unwrap();
        assert_eq!(url.with_ref(url.ref_token()), uri);
    }

    #[test]
    fn query_shallow_is_preserved_across_ref_bump() {
        let url = ArchiveUrl::parse(
            "https://forge.example.com/owner/repo/archive/25.11.tar.gz?shallow=1",
        )
        .expect("archive URL with ?shallow=1 should parse");
        assert_eq!(url.ref_token, "25.11");
        assert_eq!(url.ext, ".tar.gz");
        assert_eq!(url.tail, "?shallow=1");
        assert_eq!(
            url.with_ref("26.05"),
            "https://forge.example.com/owner/repo/archive/26.05.tar.gz?shallow=1"
        );
    }

    #[test]
    fn query_narhash_with_slash_is_preserved_byte_for_byte() {
        // A `/` inside the query must not be mistaken for a path
        // separator, and the value must round-trip untouched.
        let uri =
            "https://github.com/owner/repo/archive/v1.0.0.tar.gz?narHash=sha256-Ab%2BCd/Ef0123%3D";
        let url = ArchiveUrl::parse(uri).expect("archive URL with ?narHash should parse");
        assert_eq!(url.ref_token, "v1.0.0");
        assert_eq!(url.ext, ".tar.gz");
        assert_eq!(url.tail, "?narHash=sha256-Ab%2BCd/Ef0123%3D");
        // Same ref in, byte-identical URL out.
        assert_eq!(url.with_ref(url.ref_token()), uri);
        assert_eq!(
            url.with_ref("v2.0.0"),
            "https://github.com/owner/repo/archive/v2.0.0.tar.gz?narHash=sha256-Ab%2BCd/Ef0123%3D"
        );
    }

    #[test]
    fn parse_does_not_judge_the_ref() {
        let url = ArchiveUrl::parse("https://github.com/owner/repo/archive/main.tar.gz").unwrap();
        assert_eq!(url.ref_token, "main");
        assert_eq!(url.ref_prefix, RefPrefix::None);
    }

    #[test]
    fn rejects_non_archive_flake_refs() {
        assert!(ArchiveUrl::parse("github:owner/repo").is_none());
        assert!(ArchiveUrl::parse("git+https://github.com/owner/repo").is_none());
        assert!(ArchiveUrl::parse("https://github.com/owner/repo").is_none());
    }

    #[test]
    fn rejects_non_http_scheme() {
        assert!(ArchiveUrl::parse("ftp://github.com/owner/repo/archive/v1.0.0.tar.gz").is_none());
        assert!(
            ArchiveUrl::parse("git+https://github.com/owner/repo/archive/v1.0.0.tar.gz").is_none()
        );
    }

    #[test]
    fn rejects_unknown_extension() {
        assert!(ArchiveUrl::parse("https://github.com/owner/repo/archive/v1.0.0.tgz").is_none());
        // Bare `.tar` is intentionally not in the recognised set.
        assert!(ArchiveUrl::parse("https://github.com/owner/repo/archive/v1.0.0.tar").is_none());
    }

    #[test]
    fn rejects_missing_archive_segment() {
        // `owner/archive/<ref>` lacks the repo segment before `archive`.
        assert!(ArchiveUrl::parse("https://github.com/owner/archive/v1.0.0.tar.gz").is_none());
        assert!(
            ArchiveUrl::parse("https://github.com/owner/repo/releases/v1.0.0.tar.gz").is_none()
        );
    }

    #[test]
    fn rejects_empty_ref_token() {
        assert!(ArchiveUrl::parse("https://github.com/owner/repo/archive/.tar.gz").is_none());
    }

    #[test]
    fn rejects_multi_segment_ref_path() {
        // After `refs/tags/` a single `<ref>.<ext>` is expected; an
        // extra path component is one we do not model.
        assert!(
            ArchiveUrl::parse(
                "https://github.com/owner/repo/archive/refs/tags/nested/v1.0.0.tar.gz"
            )
            .is_none()
        );
    }

    #[test]
    fn rejects_codeload_form() {
        // codeload's `/tar.gz/<ref>` URLs are not handled.
        assert!(
            ArchiveUrl::parse("https://codeload.github.com/owner/repo/tar.gz/v1.0.0").is_none()
        );
    }
}
