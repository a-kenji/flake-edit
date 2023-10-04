use std::collections::HashMap;

use rnix::{
    ast::{
        Entry::{self, AttrpathValue},
        Expr, HasEntry,
    },
    NixLanguage, Root, SyntaxKind, SyntaxNode,
};
use rowan::GreenNode;

use crate::{input::Input, Change, State};

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
    root: SyntaxNode,
    pub inputs: HashMap<String, Input>,
    pub changes: Vec<Change>,
    commit: bool,
}

impl<'a> Walker<'a> {
    pub fn new(stream: &'a str) -> Result<Self, ()> {
        let root = Root::parse(stream).syntax();
        // let changes = Vec::new();
        let changes = vec![Change::Remove {
            id: "nixpkgs".into(),
        }];
        Ok(Self {
            stream,
            root,
            inputs: HashMap::new(),
            commit: true,
            changes,
        })
    }
    /// Traverse the toplevel `flake.nix` file.
    /// It should consist of three attribute keys:
    /// - description
    /// - inputs
    /// - outputs
    pub fn walk_toplevel(&mut self) -> Option<SyntaxNode> {
        // let expr = self.root.expr().unwrap();
        let cst = &self.root;

        if cst.kind() != SyntaxKind::NODE_ROOT {
            // TODO: handle this as an error
            panic!("Should be a topevel node.")
        } else {
            for root in cst.children() {
                // Because it is the node root this is the toplevel attribute
                for toplevel in root.children() {
                    // Match attr_sets inputs, and outputs
                    // println!("Toplevel: {}", toplevel);
                    // println!("Kind: {:?}", toplevel.kind());
                    if toplevel.kind() == SyntaxKind::NODE_ATTRPATH_VALUE {
                        for child in toplevel.children() {
                            if child.to_string() == "description" {
                                break;
                            }
                            if child.to_string() == "inputs" {
                                if let Some(replacement) =
                                    self.walk_inputs(child.next_sibling().unwrap())
                                {
                                    println!("Replacement Noode: {replacement}");
                                    let green = toplevel.green().replace_child(
                                        child.next_sibling().unwrap().index(),
                                        replacement.green().into(),
                                    );
                                    let green = toplevel.replace_with(green);
                                    let node = Root::parse(green.to_string().as_str()).syntax();
                                    println!("Noode: {node}");
                                    return Some(node);
                                }
                            }
                        }
                    } else {
                        panic!("Should be a NODE_ATTRPATH_VALUE");
                    }
                }
            }
            None
        }
    }
    pub fn walk_inputs(&mut self, node: SyntaxNode) -> Option<SyntaxNode> {
        for (i, child) in node.children().enumerate() {
            if child.kind() == SyntaxKind::NODE_ATTRPATH_VALUE {
                if let Some(replacement) = self.walk_input(&child) {
                    println!("Child Id: {}", child.index());
                    println!("Index: {}", i);
                    println!("Input replacement node: {}", node);
                    // let green = node.green().remove_child(child.index());
                    let green = node
                        .green()
                        .replace_child(child.index(), replacement.green().into());
                    let node = Root::parse(green.to_string().as_str()).syntax();
                    // let green = child.replace_with(replacement.green().into());
                    // let node = Root::parse(green.to_string().as_str()).syntax();
                    println!("Input replacement node: {}", node);
                    return Some(node);
                }
            }
        }
        None
    }
    /// Walk a single input field.
    /// Example:
    /// ```nix
    ///  flake-utils.url = "github:numtide/flake-utils";
    /// ```
    /// or
    /// ```nix
    ///  rust-overlay = {
    ///  url = "github:oxalica/rust-overlay";
    ///  inputs.nixpkgs.follows = "nixpkgs";
    ///  inputs.flake-utils.follows = "flake-utils";
    ///  };
    /// ```
    pub fn walk_input(&mut self, node: &SyntaxNode) -> Option<SyntaxNode> {
        println!("\nInput: {node}\n");
        for (i, child) in node.children().enumerate() {
            println!("Kind #:{i} {:?}", child.kind());
            if child.kind() == SyntaxKind::NODE_ATTRPATH {
                for (ii, attr) in child.children().enumerate() {
                    println!("Child of ATTRPATH #:{i} {}", child);
                    if attr.to_string() == "url" {
                        if let Some(id) = attr.prev_sibling() {
                            if (self.changes.first().unwrap().id().unwrap() == id.to_string())
                                && self.commit
                            {
                                println!("Removing: {id}");
                                // let replacement = node.green().remove_child(i);
                                let empty = Root::parse("").syntax();
                                // let green = node.replace_with(empty.green().into());
                                // let replacement = attr.replace_with(green.into());
                                // let green = rnix::NodeOrToken::Node(
                                //     rnix::Root::parse("").syntax().green().into_owned(),
                                // ).as_node();
                                // let node = Root::parse("").syntax();
                                return Some(empty);
                            }
                            println!("Id: {id}");
                        }
                        println!(
                            "This is an url:{i} {}",
                            child.parent().unwrap().next_sibling().unwrap()
                        );
                    }
                }
            }
            if child.kind() == SyntaxKind::NODE_ATTR_SET {
                for child in child.children() {
                    println!("Child of ATTRSET KIND #:{i} {:?}", child.kind());
                    println!("Child of ATTRSET #:{i} {}", child);
                    for child in child.children() {
                        println!("Child of ATTRSET KIND #:{i} {:?}", child.kind());
                        println!("Child of ATTRSET #:{i} {}", child);
                    }
                }
            }
            println!("Child #:{i} {}", child);
        }
        None
    }

    // pub fn walk_inputs(&mut self, entry: Option<Expr>) -> Option<SyntaxNode> {
    //     let entry = match entry {
    //         Some(entry) => match entry {
    //             Expr::AttrSet(attr_set) => Some(attr_set),
    //             _ => {
    //                 println!("Not matched: {:?}", entry);
    //                 None
    //             }
    //         },
    //         None => todo!(),
    //     }
    //     .unwrap();
    //     for entry in entry.entries() {
    //         println!("Entry: {}", entry);
    //     }
    //     for attr_value in entry.attrpath_values() {
    //         println!("Input Attr: {}", attr_value);
    //         self.walk_input(Entry::AttrpathValue(attr_value));
    //         // for attr in attr_value.value() {
    //         //     println!("Attr: {}", attr);
    //         // }
    //     }
    //     None
    // }
    // Walk a single input field.
    // Example:
    // ```nix
    //  flake-utils.url = "github:numtide/flake-utils";
    // ```
    // or
    // ```nix
    //  rust-overlay = {
    //  url = "github:oxalica/rust-overlay";
    //  inputs.nixpkgs.follows = "nixpkgs";
    //  inputs.flake-utils.follows = "flake-utils";
    //  };
    // ```
    // pub fn walk_input(&mut self, attrpath_values: Entry) -> Option<SyntaxNode> {
    //     let attr_values = match attrpath_values {
    //         Entry::AttrpathValue(attr_set) => Some(attr_set),
    //         _ => {
    //             println!("Not matched: {:?}", attrpath_values);
    //             None
    //         }
    //     }
    //     .unwrap();
    //
    //     // for entry in attr_values.attrpath() {
    //     println!("Individual: {}", attr_values);
    //     println!("Individual Value: {}", attr_values.value().unwrap());
    //     self.input_expr(attr_values.value());
    //     println!("Individual Attrpath: {}", attr_values.attrpath().unwrap());
    //     for attr in attr_values.attrpath().unwrap().attrs() {
    //         println!("Individual Attrpath Attrs: {}", attr);
    //     }
    //     // }
    //     None
    // }
    // pub fn input_expr(&mut self, maybe_expr: Option<Expr>) -> Option<SyntaxNode> {
    //     let entry = match maybe_expr {
    //         Some(entry) => match entry {
    //             Expr::AttrSet(attr_set) => Some(attr_set),
    //             _ => {
    //                 println!("Not matched: {:?}", entry);
    //                 println!("Not matched: {}", entry);
    //                 None
    //             }
    //         },
    //         None => todo!(),
    //     };
    //     // We should know the toplevel attr by now, for example
    //     // the flake-utils part of `flake-utils = { url = ""; };`
    //     // or also the id.
    //     if let Some(entry) = entry {
    //         println!("Individual Value: {}", entry);
    //         for entry in entry.entries() {
    //             println!("Entry: {}", entry);
    //         }
    //     }
    //     None
    // }
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
