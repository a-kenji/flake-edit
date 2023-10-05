// use url::{ParseError, Url};
use url::{ParseError, Url};

use crate::{FlakeRefType, NixUriResult};

pub(crate) struct UrlWrapper {
    url: Url,
    infer_type: bool,
    explicit_type: FlakeRefType,
}

impl UrlWrapper {
    pub(crate) fn new(url: Url) -> Self {
        Self {
            url,
            infer_type: false,
            explicit_type: FlakeRefType::None,
        }
    }
    pub(crate) fn from(input: &str) -> NixUriResult<Self> {
        let url = Url::parse(input)?;
        Ok(Self::new(url))
    }
    pub fn infer_type(&mut self, infer_type: bool) -> &mut Self {
        self.infer_type(infer_type);
        self
    }
    pub fn explicit_type(&mut self, explicit_type: FlakeRefType) -> &mut Self {
        self.explicit_type(explicit_type);
        self
    }
}
