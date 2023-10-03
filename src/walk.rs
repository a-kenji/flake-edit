use std::collections::HashMap;

use rnix::{
    ast::{
        Entry::{self, AttrpathValue},
        Expr, HasEntry,
    },
    NixLanguage, Root, SyntaxNode,
};
use rowan::GreenNode;

use crate::{input::Input, State};

// TODO:
// - parse out inputs
// - SyntaxKind(44) [inputs]
// - parse follows attribute and attrset outof the -> SyntaxKind(76) [attrset]
//
// // TODO: hopefully we won't need these codes where we are going
// NODE_STRING 63,
// NODE_IDENT 58,
// TOKEN_IDENT 44,
// TOKEN_DOT 21,
// NODE_ROOT 75,
// NODE_ATTR_SET 76,
// NODE_ATTRPATH 55,
// TOKEN_URI 49,
//

#[derive(Debug, Clone)]
pub struct Walker<'a> {
    stream: &'a str,
    root: Root,
    pub inputs: HashMap<String, Input>,
}

impl<'a> Walker<'a> {
    pub fn new(stream: &'a str) -> Result<Self, ()> {
        let root = Root::parse(stream).ok().unwrap();
        Ok(Self {
            stream,
            root,
            inputs: HashMap::new(),
        })
    }
    /// Traverse the toplevel `flake.nix` file.
    /// It should consist of three attribute keys:
    /// - description
    /// - inputs
    /// - outputs
    pub fn walk_toplevel(&mut self) {
        let expr = self.root.expr().unwrap();

        let attr_set = match expr {
            Expr::AttrSet(attr_set) => Some(attr_set),
            _ => None,
        }
        .unwrap();

        for attr in attr_set.attrpath_values() {
            if let Some(path) = attr.attrpath() {
                match path.to_string().as_str() {
                    "inputs" => {
                        println!("attr.value: {}", attr.value().unwrap());
                    }
                    // self.walk_inputs(attr.value()),
                    "description" | "outputs" => {}
                    _ => todo!("Root attribute incorrect."),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn minimal_flake() -> &'static str {
        r#"
        {
  description = "flk - a tui for your flakes.";

  inputs.nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";

  inputs.rust-overlay = {
    url = "github:oxalica/rust-overlay";
    inputs.nixpkgs.follows = "nixpkgs";
    inputs.flake-utils.follows = "flake-utils";
  }

  inputs.crane = {
    url = "github:ipetkov/crane";
    inputs.nixpkgs.follows = "nixpkgs";
    inputs.rust-overlay.follows = "rust-overlay";
    inputs.flake-utils.follows = "flake-utils";
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
    rust-overlay,
    crane,
  }:
  {};
  }
    "#
    }
    fn minimal_flake_inputs_attrs() -> &'static str {
        r#"
        {
  description = "flk - a tui for your flakes.";

  inputs = {
  nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";

  rust-overlay = {
    url = "github:oxalica/rust-overlay";
    inputs.nixpkgs.follows = "nixpkgs";
    inputs.flake-utils.follows = "flake-utils";
  };

  crane = {
    url = "github:ipetkov/crane";
    inputs.nixpkgs.follows = "nixpkgs";
    inputs.rust-overlay.follows = "rust-overlay";
    inputs.flake-utils.follows = "flake-utils";
  };
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
    rust-overlay,
    crane,
  }:
  {};
  }
    "#
    }
    fn only_inputs_flake() -> &'static str {
        r#"
        {
  inputs.nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";

  inputs.rust-overlay = {
    url = "github:oxalica/rust-overlay";
    inputs.nixpkgs.follows = "nixpkgs";
    inputs.flake-utils.follows = "flake-utils";
  };

  inputs.crane = {
    url = "github:ipetkov/crane";
    inputs.nixpkgs.follows = "nixpkgs";
    inputs.rust-overlay.follows = "rust-overlay";
    inputs.flake-utils.follows = "flake-utils";
  };

  outputs = {}:
  {};
  }
    "#
    }
    fn no_inputs_flake() -> &'static str {
        r#"
        {
  description = "flk - a tui for your flakes.";

  outputs = {
    self,
    nixpkgs,
  }:
  {};
  }
    "#
    }
}
