//! Convenience functionality for working with nix `flake.nix` references
//! (flakerefs)
//! Provides types for the generic attribute set representation, but does not parse it:
//!
//! ``no_run
//!    {
//!      type = "github";
//!      owner = "NixOS";
//!      repo = "nixpkgs";
//!    }
//! ``
//!
//! The uri syntax representation is parsed by this library:
//! example `github:a-kenji/nala`
use nom::branch::alt;
// use nom::complete::tag;
use nom::bytes::complete::{tag, take_until};
use nom::character::complete::alphanumeric0;
use nom::combinator::rest;
use nom::multi::many_m_n;
use nom::IResult;
use serde::{Deserialize, Serialize};

/// The General Flake Ref Schema
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(test, serde(deny_unknown_fields))]
struct FlakeRef {
    r#type: FlakeRefType,
    owner: Option<String>,
    repo: Option<String>,
    flake: Option<bool>,
    rev_or_ref: Option<String>,
    r#ref: Option<String>,
    attrs: FlakeRefAttributes,
}

fn parse_nix_uri(input: &str) -> IResult<&str, FlakeRef> {
    use nom::sequence::separated_pair;
    let (_, (flake_ref_type, input)) = separated_pair(alphanumeric0, tag(":"), rest)(input)?;

    // TODO: convert from string to flake_ref_type with input left over
    match std::convert::Into::<FlakeRefType>::into(flake_ref_type) {
        FlakeRefType::File(_) => todo!(),
        FlakeRefType::Git => todo!(),
        FlakeRefType::GitHub => {
            // GitHub specific! todo: branch based on FlakeRefType
            // required: repo, owner (repo/owner)
            println!("Matched Github: {}", input);
            // let (repo, input) = take_until("/")(input)?;
            // let (_, (repo, input)) = separated_pair(take_until("/"), tag("/"), rest)(input)?;
            let (input, owner_or_ref) = many_m_n(
                0,
                3,
                separated_pair(
                    take_until("/"),
                    tag("/"),
                    alt((take_until("/"), take_until("?"), rest)),
                ),
            )(input)?;

            let owner_and_rev_or_ref: Vec<&str> = owner_or_ref
                .clone()
                .into_iter()
                .flat_map(|(x, y)| vec![x, y])
                .filter(|s| !s.is_empty())
                .collect();

            let owner = owner_and_rev_or_ref[0];
            let repo = owner_and_rev_or_ref[1];
            let rev_or_ref = owner_and_rev_or_ref.get(2);

            println!("Not parsed yet: {}", input);

            let (input, param_tag) = alt((tag("?"), rest))(input)?;
            println!("Input: {}", input);
            println!("Tag: {}", param_tag);

            let mut flake_ref = FlakeRef::default();
            flake_ref.r#type(flake_ref_type.into());
            flake_ref.repo = Some(repo.into());
            flake_ref.owner = Some(owner.into());
            flake_ref.rev_or_ref = rev_or_ref.cloned().map(|s| s.clone().to_string());

            if param_tag == "?" {
                let (input, param_values) = many_m_n(
                    0,
                    5,
                    separated_pair(take_until("="), tag("="), alt((take_until("&"), rest))),
                )(input)?;
                println!("Not parsed yet: {}", input);
                println!("Params : {:?}", param_values);
                println!("Not parsed yet: {}", input);

                let mut attrs = FlakeRefAttributes::default();
                for (param, value) in param_values {
                    // Can start with "&"
                    match param.parse().unwrap() {
                        FlakeRefParam::Dir => {
                            attrs.dir(Some(value.into()));
                        }
                        FlakeRefParam::NarHash => {
                            attrs.nar_hash(Some(value.into()));
                        }
                    }
                }
                flake_ref.attrs(attrs);
                return Ok((input, flake_ref));
            } else {
                return Ok((input, flake_ref));
            }

            // Parse attributes ?attr=value concatenated by &
            //
            // let (_, (rev_or_ref, input)) =
            //     (separated_pair(take_until("/"), tag("/"), rest))(input)?;
        }
        FlakeRefType::Indirect => todo!(),
        FlakeRefType::Mercurial => todo!(),
        FlakeRefType::Path { path } => {
            // Parse till params, then take the input as an option
        }
        FlakeRefType::Sourcehut => todo!(),
        FlakeRefType::Tarball => todo!(),
        // FlakeRefType::None => todo!(),
        _ => {}
    }

    Ok((input, FlakeRef::default()))
}

impl FlakeRef {
    fn r#type(&mut self, r#type: FlakeRefType) -> &mut Self {
        self.r#type = r#type;
        self
    }
    fn owner(&mut self, owner: Option<String>) -> &mut Self {
        self.owner = owner;
        self
    }
    fn repo(&mut self, repo: Option<String>) -> &mut Self {
        self.repo = repo;
        self
    }

    fn rev_or_ref(&mut self, rev_or_ref: Option<String>) -> &mut Self {
        self.rev_or_ref = rev_or_ref;
        self
    }

    fn attrs(&mut self, attrs: FlakeRefAttributes) -> &mut Self {
        self.attrs = attrs;
        self
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(test, serde(deny_unknown_fields))]
struct FlakeRefAttributes {
    dir: Option<String>,
    #[serde(rename = "narHash")]
    nar_hash: Option<String>,
    rev: Option<String>,
    r#ref: Option<String>,
    // Not available to user
    #[serde(rename = "revCount")]
    rev_count: Option<String>,
    // Not available to user
    #[serde(rename = "lastModified")]
    last_modified: Option<String>,
}

impl FlakeRefAttributes {
    fn dir(&mut self, dir: Option<String>) -> &mut Self {
        self.dir = dir;
        self
    }

    fn nar_hash(&mut self, nar_hash: Option<String>) -> &mut Self {
        self.nar_hash = nar_hash;
        self
    }
}

pub enum FlakeRefParam {
    Dir,
    NarHash,
}

impl std::str::FromStr for FlakeRefParam {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use FlakeRefParam::*;
        match s {
            "dir" | "&dir" => Ok(Dir),
            "nar_hash" | "&nar_hash" => Ok(NarHash),
            _ => Err(()),
        }
    }
}

impl std::str::FromStr for FlakeRef {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (r#type, options) = s.split_once(':').unwrap();
        let options: Vec<&str> = options.split('/').collect();
        Ok(Self {
            r#type: r#type.into(),
            owner: Some(options[0].to_owned()),
            repo: Some(options[1].to_owned()),
            flake: None,
            attrs: FlakeRefAttributes::default(),
            rev_or_ref: None,
            r#ref: None,
        })
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
enum FlakeRefType {
    // In URL form, the schema must be file+http://, file+https:// or file+file://. If the extension doesn’t correspond to a known archive format (as defined by the tarball fetcher), then the file+ prefix can be dropped.
    File(String),
    /// Git repositories. The location of the repository is specified by the attribute `url`.
    Git,
    GitHub,
    Indirect,
    Mercurial,
    /// Path must be a directory in the filesystem containing a `flake.nix`.
    /// Path must be an absolute path.
    Path {
        path: String,
    },
    Sourcehut,
    Tarball,
    #[default]
    None,
}

impl FlakeRefType {
    /// Parse type specific information, returns the [`FlakeRefType`]
    /// and the unparsed input
    pub fn parse_type(s: &str) -> IResult<&str, FlakeRefType> {
        todo!();
        use nom::sequence::separated_pair;
        let (_, (flake_ref_type, input)) = separated_pair(alphanumeric0, tag(":"), rest)(input)?;
    }
}

impl std::str::FromStr for FlakeRefType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "github" => Ok(Self::GitHub),
            // TODO: start parser here
            "path" => Ok(Self::Path {
                path: "parse here".into(),
            }),
            "git" => Ok(Self::Git),
            _ => Err(()),
        }
    }
}
impl From<&str> for FlakeRefType {
    fn from(s: &str) -> Self {
        match s {
            "github" => Self::GitHub,
            "path" => Self::Path,
            "git" => Self::Git,
            _ => panic!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_uri_nom() {
        let uri = "github:zellij-org/zellij";
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitHub)
            .owner(Some("zellij-org".into()))
            .repo(Some("zellij".into()))
            .clone();
        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(("", flake_ref), parsed);
    }
    #[test]
    fn parse_simple_uri_ref_or_rev_nom() {
        let uri = "github:zellij-org/zellij/main";
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitHub)
            .owner(Some("zellij-org".into()))
            .repo(Some("zellij".into()))
            .rev_or_ref(Some("main".into()))
            .clone();
        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(("", flake_ref), parsed);
    }
    #[test]
    fn parse_simple_uri_ref_or_rev_attr_nom() {
        let uri = "github:zellij-org/zellij/main?dir=assets";
        let mut attrs = FlakeRefAttributes::default();
        attrs.dir(Some("assets".into()));
        let mut flake_ref = FlakeRef::default();
        flake_ref
            .r#type(FlakeRefType::GitHub)
            .owner(Some("zellij-org".into()))
            .repo(Some("zellij".into()))
            .rev_or_ref(Some("main".into()));
        flake_ref.attrs(attrs);
        let flake_ref = flake_ref.clone();

        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(("", flake_ref), parsed);
    }
    #[test]
    fn parse_simple_uri_attr_nom() {
        let uri = "github:zellij-org/zellij?dir=assets";
        let mut attrs = FlakeRefAttributes::default();
        attrs.dir(Some("assets".into()));
        let mut flake_ref = FlakeRef::default();
        flake_ref
            .r#type(FlakeRefType::GitHub)
            .owner(Some("zellij-org".into()))
            .repo(Some("zellij".into()));
        flake_ref.attrs(attrs);
        let flake_ref = flake_ref.clone();
        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(("", flake_ref), parsed);
    }
    #[test]
    fn parse_simple_uri_attr_nom_alt() {
        let uri = "github:zellij-org/zellij/?dir=assets";
        let mut attrs = FlakeRefAttributes::default();
        attrs.dir(Some("assets".into()));
        let mut flake_ref = FlakeRef::default();
        flake_ref
            .r#type(FlakeRefType::GitHub)
            .owner(Some("zellij-org".into()))
            .repo(Some("zellij".into()));
        flake_ref.attrs(attrs);
        let flake_ref = flake_ref.clone();
        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(("", flake_ref), parsed);
    }
    #[test]
    fn parse_simple_uri_attrs_nom_alt() {
        let uri = "github:zellij-org/zellij/?dir=assets&nar_hash=fakeHash256";
        let mut attrs = FlakeRefAttributes::default();
        attrs.dir(Some("assets".into()));
        attrs.nar_hash(Some("fakeHash256".into()));
        let mut flake_ref = FlakeRef::default();
        flake_ref
            .r#type(FlakeRefType::GitHub)
            .owner(Some("zellij-org".into()))
            .repo(Some("zellij".into()));
        flake_ref.attrs(attrs);
        let flake_ref = flake_ref.clone();
        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(("", flake_ref), parsed);
    }
    #[test]
    fn parse_simple_path_nom() {
        let uri = "path:/home/kenji/.config/dotfiles/";
        let mut attrs = FlakeRefAttributes::default();
        attrs.dir(Some("assets".into()));
        attrs.nar_hash(Some("fakeHash256".into()));
        let mut flake_ref = FlakeRef::default();
        flake_ref
            .r#type(FlakeRefType::Path)
            .owner(Some("zellij-org".into()))
            .repo(Some("zellij".into()));
        flake_ref.attrs(attrs);
        let flake_ref = flake_ref.clone();
        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(("", flake_ref), parsed);
    }

    // #[test]
    // fn parse_simple_uri() {
    //     let uri = "github:zellij-org/zellij";
    //     let _flake_ref: FlakeRef = uri.parse().unwrap();
    // }
    // #[test]
    // fn parse_simple_path_ok() {
    //     let uri = "path:/home/kenji/git";
    //     let _flake_ref: FlakeRef = uri.parse().unwrap();
    // }
    // #[test]
    // fn parse_simple_path() {
    //     let uri = "path:/home/kenji/git";
    //     let flake_ref: FlakeRef = uri.parse().unwrap();
    //     let mut goal = FlakeRef::default();
    //     let ref_type = FlakeRefType::default();
    //     goal.r#type(ref_type);
    //     assert_eq!(flake_ref, goal)
    // }

    // #[test]
    // fn parse_simple_uri_correctly() {
    //     let uri = "github:zellij-org/zellij";
    //     let flake_ref: FlakeRef = uri.parse().unwrap();
    //     assert_eq!(
    //         flake_ref,
    //         FlakeRef {
    //             r#type: "github".into(),
    //             owner: Some("zellij-org".into()),
    //             repo: Some("zellij".into()),
    //             flake: None,
    //             rev_or_ref: None,
    //             r#ref: None,
    //             attrs: FlakeRefAttributes::default(),
    //         }
    //     );
    // }
}
