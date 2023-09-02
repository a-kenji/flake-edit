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
//! The url syntax representation is parsed by this library:
//! example `github:a-kenji/nala`
use nom::branch::alt;
// use nom::complete::tag;
use nom::bytes::complete::{is_not, tag, take_until, take_while};
use nom::character::complete::{
    alpha1, alphanumeric0, alphanumeric1, line_ending, not_line_ending,
};
use nom::character::is_alphabetic;
use nom::combinator::{eof, rest};
use nom::multi::many_m_n;
use nom::IResult;
use serde::{Deserialize, Serialize};

/// The General Flake Ref Schema
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(test, serde(deny_unknown_fields))]
struct FlakeRef {
    r#type: FlakeRefType,
    pub owner: Option<String>,
    pub repo: Option<String>,
    flake: Option<bool>,
    rev: Option<String>,
    r#ref: Option<String>,
    attrs: FlakeRefAttributes,
}

fn parse_nix_uri<'a>(input: &'a str) -> IResult<&'a str, FlakeRef> {
    use nom::sequence::separated_pair;
    let (_, (flake_ref_type, input)) = separated_pair(alphanumeric0, tag(":"), rest)(input)?;

    //
    // let mut parser =
    // let (input, output) = parser(input)?;
    match std::convert::Into::<FlakeRefType>::into(flake_ref_type) {
        FlakeRefType::File(_) => todo!(),
        FlakeRefType::Git => todo!(),
        FlakeRefType::GitHub => {
            // GitHub specific! todo: branch based on FlakeRefType
            // required: repo, owner (repo/owner)
            println!("Matched Github: {}", input);
            // let (repo, input) = take_until("/")(input)?;
            // let (_, (repo, input)) = separated_pair(take_until("/"), tag("/"), rest)(input)?;
            let (input, owner_or_ref) =
                many_m_n(1, 4, separated_pair(take_until("/"), tag("/"), rest))(input)?;
            for owner in owner_or_ref {
                println!("Matched Github: {:?}", owner);
            }

            // let (_, (rev_or_ref, input)) =
            //     (separated_pair(take_until("/"), tag("/"), rest))(input)?;
            let mut flake_ref = FlakeRef::default();
            flake_ref.r#type(flake_ref_type.into());
            // flake_ref.repo = Some(repo.into());
            // flake_ref.owner = Some(input.into());
            // flake_ref.rev = Some(rev_or_ref.into());
            return Ok((input, flake_ref));
        }
        FlakeRefType::Indirect => todo!(),
        FlakeRefType::Mercurial => todo!(),
        FlakeRefType::Path(_) => todo!(),
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
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(test, serde(deny_unknown_fields))]
struct FlakeRefAttributes {}

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
            rev: None,
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
    Path(String),
    Sourcehut,
    Tarball,
    #[default]
    None,
}
impl std::str::FromStr for FlakeRefType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "github" => Ok(Self::GitHub),
            "path" => Ok(Self::Path(s.into())),
            "git" => Ok(Self::Git),
            _ => Err(()),
        }
    }
}
impl From<&str> for FlakeRefType {
    fn from(s: &str) -> Self {
        match s {
            "github" => Self::GitHub,
            "path" => Self::Path(s.into()),
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
        let _flake_ref: FlakeRef = uri.parse().unwrap();
        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(("", FlakeRef::default()), parsed);
    }
    #[test]
    fn parse_simple_uri_ref_or_rev_nom() {
        let uri = "github:zellij-org/zellij/main";
        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(("", FlakeRef::default()), parsed);
    }

    #[test]
    fn parse_simple_uri() {
        let uri = "github:zellij-org/zellij";
        let _flake_ref: FlakeRef = uri.parse().unwrap();
    }
    #[test]
    fn parse_simple_path_ok() {
        let uri = "path:/home/kenji/git";
        let _flake_ref: FlakeRef = uri.parse().unwrap();
    }
    // #[test]
    // fn parse_simple_path() {
    //     let uri = "path:/home/kenji/git";
    //     let flake_ref: FlakeRef = uri.parse().unwrap();
    //     let mut goal = FlakeRef::default();
    //     let ref_type = FlakeRefType::default();
    //     goal.r#type(ref_type);
    //     assert_eq!(flake_ref, goal)
    // }

    #[test]
    fn parse_simple_uri_correctly() {
        let uri = "github:zellij-org/zellij";
        let flake_ref: FlakeRef = uri.parse().unwrap();
        assert_eq!(
            flake_ref,
            FlakeRef {
                r#type: "github".into(),
                owner: Some("zellij-org".into()),
                repo: Some("zellij".into()),
                flake: None,
                rev: None,
                r#ref: None,
                attrs: FlakeRefAttributes::default(),
            }
        );
    }
}
