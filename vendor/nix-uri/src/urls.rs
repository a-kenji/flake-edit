// use url::{ParseError, Url};
use url::{ParseError, Url};

use crate::{parser::is_tarball, FlakeRef, FlakeRefType, NixUriError, NixUriResult};

pub struct UrlWrapper {
    url: Url,
    infer_type: bool,
    explicit_type: FlakeRefType,
}

impl UrlWrapper {
    pub(crate) fn new(url: Url) -> Self {
        Self {
            url,
            infer_type: true,
            explicit_type: FlakeRefType::None,
        }
    }
    pub(crate) fn from(input: &str) -> NixUriResult<Self> {
        let url = Url::parse(input)?;
        Ok(Self::new(url))
    }
    pub fn infer_type(&mut self, infer_type: bool) -> &mut Self {
        self.infer_type = infer_type;
        self
    }
    pub fn explicit_type(&mut self, explicit_type: FlakeRefType) -> &mut Self {
        self.explicit_type = explicit_type;
        self
    }
    pub fn convert_or_parse(input: &str) -> NixUriResult<FlakeRef> {
        // If default parsing fails, it might still be a `nix-uri`.
        let url = Self::from(input).ok();

        if is_tarball(input) {
            return input.parse();
        }

        if let Some(url) = url {
            match url.url.host() {
                Some(host) => {
                    let flake_ref_type = url.type_from_host(&host.to_string())?;
                    let mut flake_ref = FlakeRef::default();
                    flake_ref.r#type(flake_ref_type);
                    Ok(flake_ref.clone())
                }
                None => input.parse(),
            }
        } else {
            input.parse()
        }
    }
    fn type_from_host(&self, input: &str) -> NixUriResult<FlakeRefType> {
        match input {
            "github.com" => {
                let segments = self
                    .url
                    .path_segments()
                    .map(|c| c.collect::<Vec<_>>())
                    .ok_or(NixUriError::Error(format!(
                        "Error parsing from host: {}",
                        input
                    )))?;
                let ref_or_rev = if segments.len() > 2 {
                    Some(segments[2..].join("/"))
                } else {
                    None
                };

                if segments.len() < 2 {
                    return Err(NixUriError::Error(format!(
                        "Error parsing from host: {}",
                        input
                    )));
                }

                Ok(FlakeRefType::GitHub {
                    owner: segments[0].to_string(),
                    repo: segments[1].to_string(),
                    ref_or_rev,
                })
            }
            _ => Ok(FlakeRefType::None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn simple_url_conversion() {
        let url = "https://github.com/nixos/nixpkgs";
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::GitHub {
                owner: "nixos".into(),
                repo: "nixpkgs".into(),
                ref_or_rev: None,
            })
            .clone();
        assert_eq!(UrlWrapper::convert_or_parse(url).unwrap(), expected);
    }
    // #[test]
    // fn check_tarball_uri_conversion() {
    //     let filename = "https://github.com/NixOS/patchelf/archive/master.tar.gz";
    //     assert!(is_tarball(filename));
    // }
    // let uri = "github:nixos/nixpkgs";
    // let expected = FlakeRef::default()
    //     .r#type(FlakeRefType::GitHub {
    //         owner: "nixos".into(),
    //         repo: "nixpkgs".into(),
    //         ref_or_rev: None,
    //     })
    //     .clone();
    // let parsed: FlakeRef = uri.try_into().unwrap();
}
