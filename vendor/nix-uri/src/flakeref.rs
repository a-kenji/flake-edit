use std::{fmt::Display, path::Path};

use nom::{
    bytes::complete::{tag, take_until},
    combinator::{opt, rest},
    IResult,
};
use serde::{Deserialize, Serialize};

use crate::{
    error::{NixUriError, NixUriResult},
    parser::{parse_owner_repo_ref, parse_url_type},
};

/// The General Flake Ref Schema
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(test, serde(deny_unknown_fields))]
pub struct FlakeRef {
    pub r#type: FlakeRefType,
    flake: Option<bool>,
    pub params: FlakeRefParameters,
}

impl FlakeRef {
    pub fn new(r#type: FlakeRefType) -> Self {
        Self {
            r#type,
            ..Self::default()
        }
    }

    pub fn from<S>(input: S) -> Result<Self, NixUriError>
    where
        S: AsRef<str>,
    {
        TryInto::<Self>::try_into(input.as_ref())
    }

    pub fn r#type(&mut self, r#type: FlakeRefType) -> &mut Self {
        self.r#type = r#type;
        self
    }
    pub fn id(&self) -> Option<String> {
        self.r#type.get_id()
    }

    pub fn params(&mut self, params: FlakeRefParameters) -> &mut Self {
        self.params = params;
        self
    }
}

impl Display for FlakeRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // TODO: convert into Option
        let params = self.params.to_string();
        if params.is_empty() {
            write!(f, "{}", self.r#type)
        } else {
            write!(f, "{}?{params}", self.r#type)
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(test, serde(deny_unknown_fields))]
pub struct FlakeRefParameters {
    dir: Option<String>,
    #[serde(rename = "narHash")]
    nar_hash: Option<String>,
    rev: Option<String>,
    r#ref: Option<String>,
    branch: Option<String>,
    submodules: Option<String>,
    shallow: Option<String>,
    // Only available to certain types
    host: Option<String>,
    // Not available to user
    #[serde(rename = "revCount")]
    rev_count: Option<String>,
    // Not available to user
    #[serde(rename = "lastModified")]
    last_modified: Option<String>,
    /// Arbitrary uri parameters will be allowed during initial parsing
    /// in case they should be checked for known types run `self.check()`
    arbitrary: Vec<(String, String)>,
}

// TODO: convert into macro!
// or have params in a vec of tuples? with param and option<string>
impl Display for FlakeRefParameters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut res = String::new();
        if let Some(dir) = &self.dir {
            res.push_str("dir=");
            res.push_str(dir);
        }
        if let Some(branch) = &self.branch {
            if !res.is_empty() {
                res.push('?');
            }
            res.push_str("branch=");
            res.push_str(branch);
        }
        if let Some(host) = &self.host {
            if !res.is_empty() {
                res.push('?');
            }
            res.push_str("host=");
            res.push_str(host);
        }
        if let Some(r#ref) = &self.r#ref {
            if !res.is_empty() {
                res.push('?');
            }
            res.push_str("ref=");
            res.push_str(r#ref);
        }
        write!(f, "{res}")
    }
}

impl FlakeRefParameters {
    pub fn dir(&mut self, dir: Option<String>) -> &mut Self {
        self.dir = dir;
        self
    }

    pub fn nar_hash(&mut self, nar_hash: Option<String>) -> &mut Self {
        self.nar_hash = nar_hash;
        self
    }

    pub fn host(&mut self, host: Option<String>) -> &mut Self {
        self.host = host;
        self
    }
    pub fn rev(&mut self, rev: Option<String>) -> &mut Self {
        self.rev = rev;
        self
    }
    pub fn r#ref(&mut self, r#ref: Option<String>) -> &mut Self {
        self.r#ref = r#ref;
        self
    }

    pub fn set_dir(&mut self, dir: Option<String>) {
        self.dir = dir;
    }

    pub fn set_nar_hash(&mut self, nar_hash: Option<String>) {
        self.nar_hash = nar_hash;
    }

    pub fn set_rev(&mut self, rev: Option<String>) {
        self.rev = rev;
    }

    pub fn set_ref(&mut self, r#ref: Option<String>) {
        self.r#ref = r#ref;
    }

    pub fn set_host(&mut self, host: Option<String>) {
        self.host = host;
    }

    pub fn rev_count_mut(&mut self) -> &mut Option<String> {
        &mut self.rev_count
    }

    pub fn set_branch(&mut self, branch: Option<String>) {
        self.branch = branch;
    }

    pub fn set_submodules(&mut self, submodules: Option<String>) {
        self.submodules = submodules;
    }

    pub fn set_shallow(&mut self, shallow: Option<String>) {
        self.shallow = shallow;
    }
    pub fn add_arbitrary(&mut self, arbitrary: (String, String)) {
        self.arbitrary.push(arbitrary);
    }
}

pub enum FlakeRefParam {
    Dir,
    NarHash,
    Host,
    Ref,
    Rev,
    Branch,
    Submodules,
    Shallow,
    Arbitrary(String),
}

impl std::str::FromStr for FlakeRefParam {
    type Err = NixUriError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use FlakeRefParam::*;
        match s {
            "dir" | "&dir" => Ok(Dir),
            "nar_hash" | "&nar_hash" => Ok(NarHash),
            "host" | "&host" => Ok(Host),
            "rev" | "&rev" => Ok(Rev),
            "ref" | "&ref" => Ok(Ref),
            "branch" | "&branch" => Ok(Branch),
            "submodules" | "&submodules" => Ok(Submodules),
            "shallow" | "&shallow" => Ok(Shallow),
            arbitrary => Ok(Arbitrary(arbitrary.into())),
            // unknown => Err(NixUriError::UnknownUriParameter(unknown.into())),
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub enum FlakeRefType {
    // In URL form, the schema must be file+http://, file+https:// or file+file://. If the extension doesn’t correspond to a known archive format (as defined by the tarball fetcher), then the file+ prefix can be dropped.
    File {
        url: String,
    },
    /// Git repositories. The location of the repository is specified by the attribute
    /// `url`. The `ref` arrribute defaults to resolving the `HEAD` reference.
    /// The `rev` attribute must exist in the branch or tag specified by `ref`, defaults
    /// to `ref`.
    Git {
        url: String,
        r#type: UrlType,
    },
    GitHub {
        owner: String,
        repo: String,
        ref_or_rev: Option<String>,
    },
    GitLab {
        owner: String,
        repo: String,
        ref_or_rev: Option<String>,
    },
    Indirect {
        id: String,
        ref_or_rev: Option<String>,
    },
    // Matches `git` type, but schema is one of the following:
    // `hg+http`, `hg+https`, `hg+ssh` or `hg+file`.
    Mercurial {
        url: String,
        r#type: UrlType,
    },
    /// Path must be a directory in the filesystem containing a `flake.nix`.
    /// Path must be an absolute path.
    Path {
        path: String,
    },
    Sourcehut {
        owner: String,
        repo: String,
        ref_or_rev: Option<String>,
    },
    Tarball {
        url: String,
        r#type: UrlType,
    },
    #[default]
    None,
}

impl Display for FlakeRefType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FlakeRefType::File { url } => write!(f, "file:{url}"),
            FlakeRefType::Git { url, r#type } => todo!(),
            FlakeRefType::GitHub {
                owner,
                repo,
                ref_or_rev,
            } => {
                if let Some(ref_or_rev) = ref_or_rev {
                    write!(f, "github:{owner}/{repo}/{ref_or_rev}")
                } else {
                    write!(f, "github:{owner}/{repo}")
                }
            }
            FlakeRefType::GitLab {
                owner,
                repo,
                ref_or_rev,
            } => {
                if let Some(ref_or_rev) = ref_or_rev {
                    write!(f, "gitlab:{owner}/{repo}/{ref_or_rev}")
                } else {
                    write!(f, "gitlab:{owner}/{repo}")
                }
            }
            FlakeRefType::Indirect { id, ref_or_rev } => {
                if let Some(ref_or_rev) = ref_or_rev {
                    write!(f, "{id}/{ref_or_rev}")
                } else {
                    write!(f, "{id}")
                }
            }
            FlakeRefType::Mercurial { url, r#type } => todo!(),
            FlakeRefType::Path { path } => todo!(),
            FlakeRefType::Sourcehut {
                owner,
                repo,
                ref_or_rev,
            } => {
                if let Some(ref_or_rev) = ref_or_rev {
                    write!(f, "sourcehut:{owner}/{repo}/{ref_or_rev}")
                } else {
                    write!(f, "sourcehut:{owner}/{repo}")
                }
            }
            // TODO: alternate tarball representation
            FlakeRefType::Tarball { url, r#type } => {
                write!(f, "file:{url}")
            }
            FlakeRefType::None => todo!(),
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum UrlType {
    #[default]
    None,
    Https,
    Ssh,
    File,
}

impl TryFrom<&str> for UrlType {
    type Error = NixUriError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        use UrlType::*;
        match value {
            "" => Ok(None),
            "https" => Ok(Https),
            "ssh" => Ok(Ssh),
            "file" => Ok(File),
            err => Err(NixUriError::UnknownUrlType(err.into())),
        }
    }
}

impl Display for UrlType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UrlType::None => write!(f, "No Url Type Specified"),
            UrlType::Https => write!(f, ""),
            UrlType::Ssh => write!(f, ""),
            UrlType::File => write!(f, ""),
        }
    }
}

impl FlakeRefType {
    /// Parse type specific information, returns the [`FlakeRefType`]
    /// and the unparsed input
    pub fn parse_type(input: &str) -> NixUriResult<FlakeRefType> {
        use nom::sequence::separated_pair;
        let (_, maybe_explicit_type) = opt(separated_pair(
            take_until::<&str, &str, (&str, nom::error::ErrorKind)>(":"),
            tag(":"),
            rest,
        ))(input)?;
        if let Some((flake_ref_type, input)) = maybe_explicit_type {
            match flake_ref_type {
                "github" => {
                    let (input, owner_and_repo_or_ref) = parse_owner_repo_ref(input)?;
                    let owner =
                        owner_and_repo_or_ref
                            .first()
                            .ok_or(NixUriError::MissingTypeParameter(
                                flake_ref_type.into(),
                                "owner".into(),
                            ))?;
                    let repo =
                        owner_and_repo_or_ref
                            .get(1)
                            .ok_or(NixUriError::MissingTypeParameter(
                                flake_ref_type.into(),
                                "repo".into(),
                            ))?;
                    let flake_ref_type = FlakeRefType::GitHub {
                        owner: owner.to_string(),
                        repo: repo.to_string(),
                        ref_or_rev: owner_and_repo_or_ref.get(2).map(|s| s.to_string()),
                    };
                    Ok(flake_ref_type)
                }
                "gitlab" => {
                    let (input, owner_and_repo_or_ref) = parse_owner_repo_ref(input)?;
                    let owner =
                        owner_and_repo_or_ref
                            .first()
                            .ok_or(NixUriError::MissingTypeParameter(
                                flake_ref_type.into(),
                                "owner".into(),
                            ))?;
                    let repo =
                        owner_and_repo_or_ref
                            .get(1)
                            .ok_or(NixUriError::MissingTypeParameter(
                                flake_ref_type.into(),
                                "repo".into(),
                            ))?;
                    let flake_ref_type = FlakeRefType::GitLab {
                        owner: owner.to_string(),
                        repo: repo.to_string(),
                        ref_or_rev: owner_and_repo_or_ref.get(2).map(|s| s.to_string()),
                    };
                    Ok(flake_ref_type)
                }
                "sourcehut" => {
                    let (input, owner_and_repo_or_ref) = parse_owner_repo_ref(input)?;
                    let owner =
                        owner_and_repo_or_ref
                            .first()
                            .ok_or(NixUriError::MissingTypeParameter(
                                flake_ref_type.into(),
                                "owner".into(),
                            ))?;
                    let repo =
                        owner_and_repo_or_ref
                            .get(1)
                            .ok_or(NixUriError::MissingTypeParameter(
                                flake_ref_type.into(),
                                "repo".into(),
                            ))?;
                    let flake_ref_type = FlakeRefType::Sourcehut {
                        owner: owner.to_string(),
                        repo: repo.to_string(),
                        ref_or_rev: owner_and_repo_or_ref.get(2).map(|s| s.to_string()),
                    };
                    Ok(flake_ref_type)
                }
                "path" => {
                    // TODO: check if path is an absolute path, if not error
                    let path = Path::new(input);
                    // TODO: make this check configurable for cli usage
                    if !path.is_absolute() || input.contains(']') || input.contains('[') {
                        return Err(NixUriError::NotAbsolute(input.into()));
                    }
                    if input.contains('#') || input.contains('?') {
                        return Err(NixUriError::PathCharacter(input.into()));
                    }
                    let flake_ref_type = FlakeRefType::Path { path: input.into() };
                    Ok(flake_ref_type)
                }

                _ => {
                    if flake_ref_type.starts_with("git+") {
                        let url_type = parse_url_type(flake_ref_type)?;
                        let (input, _tag) =
                            opt(tag::<&str, &str, (&str, nom::error::ErrorKind)>("//"))(input)?;
                        let flake_ref_type = FlakeRefType::Git {
                            url: input.into(),
                            r#type: url_type,
                        };
                        Ok(flake_ref_type)
                    } else if flake_ref_type.starts_with("hg+") {
                        let url_type = parse_url_type(flake_ref_type)?;
                        let (input, _tag) =
                            tag::<&str, &str, (&str, nom::error::ErrorKind)>("//")(input)?;
                        let flake_ref_type = FlakeRefType::Mercurial {
                            url: input.into(),
                            r#type: url_type,
                        };
                        Ok(flake_ref_type)
                    } else {
                        Err(NixUriError::UnknownUriType(flake_ref_type.into()))
                    }
                }
            }
        } else {
            // Implicit types can be paths, indirect flake_refs, or uri's.
            if input.starts_with('/') || input == "." {
                let flake_ref_type = FlakeRefType::Path { path: input.into() };
                let path = Path::new(input);
                // TODO: make this check configurable for cli usage
                if !path.is_absolute()
                    || input.contains(']')
                    || input.contains('[')
                    || !input.chars().all(|c| c.is_ascii())
                {
                    return Err(NixUriError::NotAbsolute(input.into()));
                }
                if input.contains('#') || input.contains('?') {
                    return Err(NixUriError::PathCharacter(input.into()));
                }
                return Ok(flake_ref_type);
            }
            //TODO: parse uri
            let (input, owner_and_repo_or_ref) = parse_owner_repo_ref(input)?;
            if !owner_and_repo_or_ref.is_empty() {
                let id = if let Some(id) = owner_and_repo_or_ref.first() {
                    id
                } else {
                    input
                };
                if !id
                    .chars()
                    .all(|c| c.is_ascii_alphabetic() || c.is_control())
                    || id.is_empty()
                {
                    return Err(NixUriError::InvalidUrl(input.into()));
                }
                let flake_ref_type = FlakeRefType::Indirect {
                    id: id.to_string(),
                    ref_or_rev: owner_and_repo_or_ref.get(1).map(|s| s.to_string()),
                };
                Ok(flake_ref_type)
            } else {
                let (_input, owner_and_repo_or_ref) = parse_owner_repo_ref(input)?;
                let id = if let Some(id) = owner_and_repo_or_ref.first() {
                    id
                } else {
                    input
                };
                if !id.chars().all(|c| c.is_ascii_alphabetic()) || id.is_empty() {
                    return Err(NixUriError::InvalidUrl(input.into()));
                }
                Ok(FlakeRefType::Indirect {
                    id: id.to_string(),
                    ref_or_rev: owner_and_repo_or_ref.get(1).map(|s| s.to_string()),
                })
            }
        }
    }
    /// Extract a common identifier from it's [`FlakeRefType`] variant.
    pub(crate) fn get_id(&self) -> Option<String> {
        match self {
            FlakeRefType::File { url } => None,
            FlakeRefType::Git { url, r#type } => None,
            FlakeRefType::GitHub {
                owner,
                repo,
                ref_or_rev,
            } => Some(repo.to_string()),
            FlakeRefType::GitLab {
                owner,
                repo,
                ref_or_rev,
            } => Some(repo.to_string()),
            FlakeRefType::Indirect { id, ref_or_rev } => None,
            FlakeRefType::Mercurial { url, r#type } => None,
            FlakeRefType::Path { path } => None,
            FlakeRefType::Sourcehut {
                owner,
                repo,
                ref_or_rev,
            } => Some(repo.to_string()),
            FlakeRefType::Tarball { url, r#type } => None,
            FlakeRefType::None => None,
        }
    }
    pub fn ref_or_rev(&mut self, ref_or_rev_alt: Option<String>) -> Result<(), NixUriError> {
        match self {
            FlakeRefType::GitHub { ref_or_rev, .. }
            | FlakeRefType::GitLab { ref_or_rev, .. }
            | FlakeRefType::Indirect { ref_or_rev, .. }
            | FlakeRefType::Sourcehut { ref_or_rev, .. } => {
                *ref_or_rev = ref_or_rev_alt;
            }
            // TODO: return a proper error, if ref_or_rev is tried to be specified
            FlakeRefType::Mercurial { .. }
            | FlakeRefType::Path { .. }
            | FlakeRefType::Tarball { .. }
            | FlakeRefType::File { .. }
            | FlakeRefType::Git { .. }
            | FlakeRefType::None => todo!(),
        }
        Ok(())
    }
}

impl TryFrom<&str> for FlakeRef {
    type Error = NixUriError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        use crate::parser::parse_nix_uri;
        parse_nix_uri(value)
    }
}

impl std::str::FromStr for FlakeRef {
    type Err = NixUriError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use crate::parser::parse_nix_uri;
        parse_nix_uri(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{parse_nix_uri, parse_params};

    #[test]
    fn parse_simple_uri() {
        let uri = "github:nixos/nixpkgs";
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::GitHub {
                owner: "nixos".into(),
                repo: "nixpkgs".into(),
                ref_or_rev: None,
            })
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }

    #[test]
    fn parse_simple_uri_parsed() {
        let uri = "github:zellij-org/zellij";
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::GitHub {
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_or_rev: None,
            })
            .clone();
        let parsed: FlakeRef = uri.parse().unwrap();
        assert_eq!(expected, parsed);
    }

    #[test]
    fn parse_simple_uri_nom() {
        let uri = "github:zellij-org/zellij";
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitHub {
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_or_rev: None,
            })
            .clone();
        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(flake_ref, parsed);
    }
    #[test]
    fn parse_simple_uri_nom_params() {
        let uri = "github:zellij-org/zellij";
        let flake_attrs = None;
        let parsed = parse_params(uri).unwrap();
        assert_eq!(("github:zellij-org/zellij", flake_attrs), parsed);
    }
    #[test]
    fn parse_simple_uri_attr_nom_params() {
        let uri = "github:zellij-org/zellij?dir=assets";
        let mut flake_attrs = FlakeRefParameters::default();
        flake_attrs.dir(Some("assets".into()));
        let parsed = parse_params(uri).unwrap();
        assert_eq!(("github:zellij-org/zellij", Some(flake_attrs)), parsed);
    }

    #[test]
    fn parse_simple_uri_ref_or_rev_nom() {
        let uri = "github:zellij-org/zellij/main";
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitHub {
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_or_rev: Some("main".into()),
            })
            .clone();
        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(flake_ref, parsed);
    }
    #[test]
    fn parse_simple_uri_ref_or_rev_attr_nom() {
        let uri = "github:zellij-org/zellij/main?dir=assets";
        let mut params = FlakeRefParameters::default();
        params.dir(Some("assets".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitHub {
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_or_rev: Some("main".into()),
            })
            .params(params)
            .clone();

        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(flake_ref, parsed);
    }
    #[test]
    fn parse_simple_uri_attr_nom() {
        let uri = "github:zellij-org/zellij?dir=assets";
        let mut params = FlakeRefParameters::default();
        params.dir(Some("assets".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitHub {
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_or_rev: None,
            })
            .params(params)
            .clone();
        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(flake_ref, parsed);
    }
    #[test]
    fn parse_simple_uri_attr_nom_alt() {
        let uri = "github:zellij-org/zellij/?dir=assets";
        let mut params = FlakeRefParameters::default();
        params.dir(Some("assets".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitHub {
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_or_rev: None,
            })
            .params(params)
            .clone();
        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(flake_ref, parsed);
    }
    #[test]
    fn parse_simple_uri_params_nom_alt() {
        let uri = "github:zellij-org/zellij/?dir=assets&nar_hash=fakeHash256";
        let mut params = FlakeRefParameters::default();
        params.dir(Some("assets".into()));
        params.nar_hash(Some("fakeHash256".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitHub {
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_or_rev: None,
            })
            .params(params)
            .clone();
        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(flake_ref, parsed);
    }
    #[test]
    fn parse_simple_path_nom() {
        let uri = "path:/home/kenji/.config/dotfiles/";
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::Path {
                path: "/home/kenji/.config/dotfiles/".into(),
            })
            .clone();
        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(flake_ref, parsed);
    }
    #[test]
    fn parse_simple_path_params_nom() {
        let uri = "path:/home/kenji/.config/dotfiles/?dir=assets";
        let mut params = FlakeRefParameters::default();
        params.dir(Some("assets".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::Path {
                path: "/home/kenji/.config/dotfiles/".into(),
            })
            .params(params)
            .clone();
        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(flake_ref, parsed);
    }
    #[test]
    fn parse_gitlab_simple() {
        let uri = "gitlab:veloren/veloren";
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitLab {
                owner: "veloren".into(),
                repo: "veloren".into(),
                ref_or_rev: None,
            })
            .clone();
        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(flake_ref, parsed);
    }
    #[test]
    fn parse_gitlab_simple_ref_or_rev() {
        let uri = "gitlab:veloren/veloren/master";
        let parsed = parse_nix_uri(uri).unwrap();
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitLab {
                owner: "veloren".into(),
                repo: "veloren".into(),
                ref_or_rev: Some("master".into()),
            })
            .clone();
        assert_eq!(flake_ref, parsed);
    }
    #[test]
    fn parse_gitlab_simple_ref_or_rev_alt() {
        let uri = "gitlab:veloren/veloren/19742bb9300fb0be9fdc92f30766c95230a8a371";
        let parsed = crate::parser::parse_nix_uri(uri).unwrap();
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitLab {
                owner: "veloren".into(),
                repo: "veloren".into(),
                ref_or_rev: Some("19742bb9300fb0be9fdc92f30766c95230a8a371".into()),
            })
            .clone();
        assert_eq!(flake_ref, parsed);
    }
    // TODO: replace / with %2F
    // #[test]
    // fn parse_gitlab_nested_subgroup() {
    //     let uri = "gitlab:veloren%2Fdev/rfcs";
    //     let parsed = parse_nix_uri(uri).unwrap();
    //     let flake_ref = FlakeRef::default()
    //         .r#type(FlakeRefType::GitLab {
    //             owner: "veloren".into(),
    //             repo: "dev".into(),
    //             ref_or_rev: Some("rfcs".to_owned()),
    //         })
    //         .clone();
    //     assert_eq!(("", flake_ref), parsed);
    // }
    #[test]
    fn parse_gitlab_simple_host_param() {
        let uri = "gitlab:openldap/openldap?host=git.openldap.org";
        let parsed = crate::parser::parse_nix_uri(uri).unwrap();
        let mut params = FlakeRefParameters::default();
        params.host(Some("git.openldap.org".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitLab {
                owner: "openldap".into(),
                repo: "openldap".into(),
                ref_or_rev: None,
            })
            .params(params)
            .clone();
        assert_eq!(flake_ref, parsed);
    }
    #[test]
    fn parse_git_and_https_simple() {
        let uri = "git+https://git.somehost.tld/user/path";
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Git {
                url: "git.somehost.tld/user/path".into(),
                r#type: UrlType::Https,
            })
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    fn parse_git_and_https_params() {
        let uri = "git+https://git.somehost.tld/user/path?ref=branch&rev=fdc8ef970de2b4634e1b3dca296e1ed918459a9e";
        let mut params = FlakeRefParameters::default();
        params.r#ref(Some("branch".into()));
        params.rev(Some("fdc8ef970de2b4634e1b3dca296e1ed918459a9e".into()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Git {
                url: "git.somehost.tld/user/path".into(),
                r#type: UrlType::Https,
            })
            .params(params)
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    fn parse_git_and_file_params() {
        let uri = "git+file:///nix/nixpkgs?ref=upstream/nixpkgs-unstable";
        let mut params = FlakeRefParameters::default();
        params.r#ref(Some("upstream/nixpkgs-unstable".into()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Git {
                url: "/nix/nixpkgs".into(),
                r#type: UrlType::File,
            })
            .params(params)
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    fn parse_git_and_file_simple() {
        let uri = "git+file:///nix/nixpkgs";
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Git {
                url: "/nix/nixpkgs".into(),
                r#type: UrlType::File,
            })
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    // TODO: is this correct?
    // git+file:/home/user/forked-flake?branch=feat/myNewFeature
    fn parse_git_and_file_params_alt() {
        let uri = "git+file:///home/user/forked-flake?branch=feat/myNewFeature";
        let mut params = FlakeRefParameters::default();
        params.set_branch(Some("feat/myNewFeature".into()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Git {
                url: "/home/user/forked-flake".into(),
                r#type: UrlType::File,
            })
            .params(params)
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    fn parse_github_simple_tag_non_alphabetic_params() {
        let uri = "github:smunix/MyST-Parser?ref=fix.hls-docutils";
        let mut params = FlakeRefParameters::default();
        params.set_ref(Some("fix.hls-docutils".to_owned()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::GitHub {
                owner: "smunix".into(),
                repo: "MyST-Parser".into(),
                ref_or_rev: None,
            })
            .params(params)
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    fn parse_github_simple_tag() {
        let uri = "github:cachix/devenv/v0.5";
        let mut params = FlakeRefParameters::default();
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::GitHub {
                owner: "cachix".into(),
                repo: "devenv".into(),
                ref_or_rev: Some("v0.5".into()),
            })
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    fn parse_git_and_file_params_alt_branch() {
        let uri = "git+file:///home/user/forked-flake?branch=feat/myNewFeature";
        let mut params = FlakeRefParameters::default();
        params.set_branch(Some("feat/myNewFeature".into()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Git {
                url: "/home/user/forked-flake".into(),
                r#type: UrlType::File,
            })
            .params(params)
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    fn parse_gitlab_with_host_params_alt() {
        let uri = "gitlab:fpottier/menhir/20201216?host=gitlab.inria.fr";
        let mut params = FlakeRefParameters::default();
        params.set_host(Some("gitlab.inria.fr".into()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::GitLab {
                owner: "fpottier".to_owned(),
                repo: "menhir".to_owned(),
                ref_or_rev: Some("20201216".to_owned()),
            })
            .params(params)
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    fn parse_git_and_https_params_submodules() {
        let uri = "git+https://www.github.com/ocaml/ocaml-lsp?submodules=1";
        let mut params = FlakeRefParameters::default();
        params.set_submodules(Some("1".into()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Git {
                url: "www.github.com/ocaml/ocaml-lsp".to_owned(),
                r#type: UrlType::Https,
            })
            .params(params)
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    fn parse_marcurial_and_https_simpe_uri() {
        let uri = "hg+https://www.github.com/ocaml/ocaml-lsp";
        let mut params = FlakeRefParameters::default();
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Mercurial {
                url: "www.github.com/ocaml/ocaml-lsp".to_owned(),
                r#type: UrlType::Https,
            })
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    #[should_panic]
    fn parse_git_and_https_params_submodules_wrong_type() {
        let uri = "gt+https://www.github.com/ocaml/ocaml-lsp?submodules=1";
        let mut params = FlakeRefParameters::default();
        params.set_submodules(Some("1".into()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Git {
                url: "www.github.com/ocaml/ocaml-lsp".to_owned(),
                r#type: UrlType::Https,
            })
            .params(params)
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    fn parse_git_and_file_shallow() {
        let uri = "git+file:/path/to/repo?shallow=1";
        let mut params = FlakeRefParameters::default();
        params.set_shallow(Some("1".into()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Git {
                url: "/path/to/repo".to_owned(),
                r#type: UrlType::File,
            })
            .params(params)
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    // TODO: allow them with an optional cli parser
    // #[test]
    // fn parse_simple_path_uri_indirect() {
    //     let uri = "path:../.";
    //     let expected = FlakeRef::default()
    //         .r#type(FlakeRefType::Path {
    //             path: "../.".to_owned(),
    //         })
    //         .clone();
    //     let parsed: FlakeRef = uri.try_into().unwrap();
    //     assert_eq!(expected, parsed);
    // }
    // TODO: allow them with an optional cli parser
    // #[test]
    // fn parse_simple_path_uri_indirect_local() {
    //     let uri = "path:.";
    //     let expected = FlakeRef::default()
    //         .r#type(FlakeRefType::Path {
    //             path: ".".to_owned(),
    //         })
    //         .clone();
    //     let parsed: FlakeRef = uri.try_into().unwrap();
    //     assert_eq!(expected, parsed);
    // }
    #[test]
    fn parse_simple_uri_sourcehut() {
        let uri = "sourcehut:~misterio/nix-colors";
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Sourcehut {
                owner: "~misterio".to_owned(),
                repo: "nix-colors".to_owned(),
                ref_or_rev: None,
            })
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    fn parse_simple_uri_sourcehut_rev() {
        let uri = "sourcehut:~misterio/nix-colors/main";
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Sourcehut {
                owner: "~misterio".to_owned(),
                repo: "nix-colors".to_owned(),
                ref_or_rev: Some("main".to_owned()),
            })
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    fn parse_simple_uri_sourcehut_host_param() {
        let uri = "sourcehut:~misterio/nix-colors?host=git.example.org";
        let mut params = FlakeRefParameters::default();
        params.set_host(Some("git.example.org".into()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Sourcehut {
                owner: "~misterio".to_owned(),
                repo: "nix-colors".to_owned(),
                ref_or_rev: None,
            })
            .params(params)
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    fn parse_simple_uri_sourcehut_ref() {
        let uri = "sourcehut:~misterio/nix-colors/182b4b8709b8ffe4e9774a4c5d6877bf6bb9a21c";
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Sourcehut {
                owner: "~misterio".to_owned(),
                repo: "nix-colors".to_owned(),
                ref_or_rev: Some("182b4b8709b8ffe4e9774a4c5d6877bf6bb9a21c".to_owned()),
            })
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    fn parse_simple_uri_sourcehut_ref_params() {
        let uri =
            "sourcehut:~misterio/nix-colors/21c1a380a6915d890d408e9f22203436a35bb2de?host=hg.sr.ht";
        let mut params = FlakeRefParameters::default();
        params.set_host(Some("hg.sr.ht".into()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Sourcehut {
                owner: "~misterio".to_owned(),
                repo: "nix-colors".to_owned(),
                ref_or_rev: Some("21c1a380a6915d890d408e9f22203436a35bb2de".to_owned()),
            })
            .params(params)
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    fn display_simple_sourcehut_uri_ref_or_rev() {
        let expected = "sourcehut:~misterio/nix-colors/21c1a380a6915d890d408e9f22203436a35bb2de";
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::Sourcehut {
                owner: "~misterio".to_owned(),
                repo: "nix-colors".to_owned(),
                ref_or_rev: Some("21c1a380a6915d890d408e9f22203436a35bb2de".to_owned()),
            })
            .to_string();
        assert_eq!(expected, flake_ref);
    }
    #[test]
    fn display_simple_sourcehut_uri_ref_or_rev_host_param() {
        let expected =
            "sourcehut:~misterio/nix-colors/21c1a380a6915d890d408e9f22203436a35bb2de?host=hg.sr.ht";
        let mut params = FlakeRefParameters::default();
        params.set_host(Some("hg.sr.ht".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::Sourcehut {
                owner: "~misterio".to_owned(),
                repo: "nix-colors".to_owned(),
                ref_or_rev: Some("21c1a380a6915d890d408e9f22203436a35bb2de".to_owned()),
            })
            .params(params)
            .to_string();
        assert_eq!(expected, flake_ref);
    }

    #[test]
    fn parse_simple_path_uri_indirect_absolute_without_prefix() {
        let uri = "/home/kenji/git";
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Path {
                path: "/home/kenji/git".to_owned(),
            })
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    fn parse_simple_path_uri_indirect_absolute_without_prefix_with_params() {
        let uri = "/home/kenji/git?dir=dev";
        let mut params = FlakeRefParameters::default();
        params.set_dir(Some("dev".into()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Path {
                path: "/home/kenji/git".to_owned(),
            })
            .params(params)
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }

    // TODO: allow them with an optional cli parser
    // #[test]
    // fn parse_simple_path_uri_indirect_local_without_prefix() {
    //     let uri = ".";
    //     let expected = FlakeRef::default()
    //         .r#type(FlakeRefType::Path {
    //             path: ".".to_owned(),
    //         })
    //         .clone();
    //     let parsed: FlakeRef = uri.try_into().unwrap();
    //     assert_eq!(expected, parsed);
    // }

    #[test]
    fn parse_wrong_git_uri_extension_type() {
        let uri = "git+(:z";
        let expected = NixUriError::UnknownUrlType("(".into());
        let parsed: NixUriResult<FlakeRef> = uri.try_into();
        assert_eq!(expected, parsed.unwrap_err());
    }
    #[test]
    fn parse_github_missing_parameter() {
        let uri = "github:";
        let expected = NixUriError::MissingTypeParameter("github".into(), ("owner".into()));
        let parsed: NixUriResult<FlakeRef> = uri.try_into();
        assert_eq!(expected, parsed.unwrap_err());
    }
    #[test]
    fn parse_github_missing_parameter_repo() {
        let uri = "github:nixos/";
        assert_eq!(
            uri.parse::<FlakeRef>(),
            Err(NixUriError::MissingTypeParameter(
                "github".into(),
                ("repo".into())
            ))
        );
    }
    #[test]
    fn parse_github_starts_with_whitespace() {
        let uri = " github:nixos/nixpkgs";
        assert_eq!(
            uri.parse::<FlakeRef>(),
            Err(NixUriError::InvalidUrl(uri.into()))
        );
    }
    #[test]
    fn parse_github_ends_with_whitespace() {
        let uri = "github:nixos/nixpkgs ";
        assert_eq!(
            uri.parse::<FlakeRef>(),
            Err(NixUriError::InvalidUrl(uri.into()))
        );
    }
    #[test]
    fn parse_empty_invalid_url() {
        let uri = "";
        assert_eq!(
            uri.parse::<FlakeRef>(),
            Err(NixUriError::InvalidUrl(uri.into()))
        );
    }
    #[test]
    fn parse_empty_trim_invalid_url() {
        let uri = "  ";
        assert_eq!(
            uri.parse::<FlakeRef>(),
            Err(NixUriError::InvalidUrl(uri.into()))
        );
    }
    #[test]
    fn parse_slash_trim_invalid_url() {
        let uri = "   /   ";
        assert_eq!(
            uri.parse::<FlakeRef>(),
            Err(NixUriError::InvalidUrl(uri.into()))
        );
    }
    #[test]
    fn parse_double_trim_invalid_url() {
        let uri = "   :   ";
        assert_eq!(
            uri.parse::<FlakeRef>(),
            Err(NixUriError::InvalidUrl(uri.into()))
        );
    }

    // #[test]
    // fn parse_simple_indirect() {
    //     let uri = "nixos/nixpkgs";
    //     let expected = FlakeRef::default()
    //         .r#type(FlakeRefType::Indirect {
    //             id: "nixos/nixpkgs".to_owned(),
    //             ref_or_rev: None,
    //         })
    //         .clone();
    //     let parsed: FlakeRef = uri.try_into().unwrap();
    //     assert_eq!(expected, parsed);
    // }

    // TODO: indirect uris
    // #[test]
    // fn parse_simple_tarball() {
    //     let uri = "https://hackage.haskell.org/package/lsp-test-0.14.0.3/lsp-test-0.14.0.3.tar.gz";
    //     let mut params = FlakeRefParameters::default();
    //     let expected = FlakeRef::default()
    //         .r#type(FlakeRefType::Tarball {
    //             id: "nixpkgs".to_owned(),
    //             ref_or_rev: Some("nixos-23.05".to_owned()),
    //         })
    //         .params(params)
    //         .clone();
    //     let parsed: FlakeRef = uri.try_into().unwrap();
    //     assert_eq!(expected, parsed);
    // }
}
