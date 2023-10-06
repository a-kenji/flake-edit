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
    pub root: SyntaxNode,
    pub inputs: HashMap<String, Input>,
    pub changes: Vec<Change>,
    commit: bool,
}

impl<'a> Walker<'a> {
    pub fn new(stream: &'a str) -> Result<Self, ()> {
        let root = Root::parse(stream).syntax();
        let changes = Vec::new();
        // let changes = vec![
        //     Change::Add {
        //         id: Some("nixpkgs".into()),
        //         uri: Some("github:nixos/nixpkgs".into()),
        //     },
        //     Change::Remove {
        //         id: "nixpkgs".into(),
        //     },
        // ];
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
                    tracing::debug!("Toplevel: {}", toplevel);
                    tracing::debug!("Kind: {:?}", toplevel.kind());
                    if toplevel.kind() == SyntaxKind::NODE_ATTRPATH_VALUE {
                        for child in toplevel.children() {
                            if child.to_string() == "description" {
                                break;
                            }
                            if child.to_string() == "inputs" {
                                if let Some(replacement) =
                                    self.walk_inputs(child.next_sibling().unwrap())
                                {
                                    tracing::debug!("Replacement Noode: {replacement}");
                                    let green = toplevel.green().replace_child(
                                        child.next_sibling().unwrap().index(),
                                        replacement.green().into(),
                                    );
                                    let green = toplevel.replace_with(green);
                                    let node = Root::parse(green.to_string().as_str()).syntax();
                                    tracing::debug!("Noode: {node}");
                                    return Some(node);
                                }
                            } else if child.to_string().starts_with("inputs") {
                                self.walk_inputs(child);
                                // for input in child.children() {
                                //     println!("Input Kind: {:?}", input.kind());
                                //     println!("Input: {}", input);
                                // }
                                // for input in child.next_sibling().unwrap().children() {
                                //     println!(
                                //         "Input Sibling Kind Child of {child}: {:?}",
                                //         input.kind()
                                //     );
                                //     println!("Input Sibling Child of {child}: {}", input);
                                // }
                                // self.walk_inputs(child);
                                // if let Some(input) = child.next_sibling() {
                                //     if let Some(first_child) = input.first_child() {
                                //         if let Some(replacement) = self.walk_inputs(first_child) {
                                //             println!("Replacement: {}", replacement);
                                //         }
                                //     }
                                // }
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
        for child in node.children_with_tokens() {
            tracing::debug!("Inputs Child Kind: {:?}", child.kind());
            tracing::debug!("Inputs Child: {child}");
            tracing::debug!("Inputs Child Len: {}", child.to_string().len());
            match child.kind() {
                SyntaxKind::NODE_ATTRPATH_VALUE => {
                    if let Some(replacement) = self.walk_input(child.as_node().unwrap()) {
                        tracing::debug!("Child Id: {}", child.index());
                        tracing::debug!("Input replacement node: {}", node);
                        let mut green = node
                            .green()
                            .replace_child(child.index(), replacement.green().into());
                        if replacement.text().is_empty() {
                            let prev = child.prev_sibling_or_token();
                            if let Some(prev) = prev {
                                if let SyntaxKind::TOKEN_WHITESPACE = prev.kind() {
                                    green = green.remove_child(prev.index());
                                }
                            } else if let Some(next) = child.next_sibling_or_token() {
                                if let SyntaxKind::TOKEN_WHITESPACE = next.kind() {
                                    green = green.remove_child(next.index());
                                }
                            }
                        }
                        let node = Root::parse(green.to_string().as_str()).syntax();
                        return Some(node);
                    } else if let Some(change) = self.changes.first() {
                        if (change.id().is_some()) && self.commit {
                            if let Change::Add { id, uri } = change {
                                let uri = Root::parse(&format!(
                                    "{} = \"{}\";",
                                    id.clone().unwrap(),
                                    uri.clone().unwrap(),
                                ))
                                .syntax();
                                let mut green =
                                    node.green().insert_child(child.index(), uri.green().into());
                                let prev = child.prev_sibling_or_token().unwrap();
                                tracing::debug!("Token:{}", prev);
                                tracing::debug!("Token Kind: {:?}", prev.kind());
                                if prev.kind() == SyntaxKind::TOKEN_WHITESPACE {
                                    let whitespace =
                                        Root::parse(prev.as_token().unwrap().green().text())
                                            .syntax();
                                    green = green
                                        .insert_child(child.index() + 1, whitespace.green().into());
                                }
                                // let green =
                                // green.insert_child(child.index() + 1, whitespace.green().into());
                                tracing::debug!("green: {}", green);
                                tracing::debug!("node: {}", node);
                                tracing::debug!("node kind: {:?}", node.kind());
                                let node = Root::parse(green.to_string().as_str()).syntax();
                                return Some(node);
                            }
                        }
                    }
                }
                SyntaxKind::NODE_IDENT => {
                    tracing::debug!("Node PATH: {}", child);
                    if child.to_string() == "inputs" {
                        if let Some(next_sibling) = child.as_node().unwrap().next_sibling() {
                            match next_sibling.kind() {
                                SyntaxKind::NODE_IDENT => {
                                    tracing::debug!("NODE_IDENT input: {}", next_sibling);
                                    tracing::debug!("NODE_IDENT input: {}", next_sibling);
                                    if let Some(url_id) = next_sibling.next_sibling() {
                                        match url_id.kind() {
                                            SyntaxKind::NODE_IDENT => {
                                                if url_id.to_string() == "url" {
                                                    if let Some(url) = child
                                                        .as_node()
                                                        .unwrap()
                                                        .parent()
                                                        .unwrap()
                                                        .next_sibling()
                                                    {
                                                        tracing::debug!(
                                                            "This is an url from {} - {}",
                                                            next_sibling,
                                                            url
                                                        );
                                                        let mut input =
                                                            Input::new(next_sibling.to_string());
                                                        input.url = url.to_string();
                                                        self.inputs.insert(
                                                            next_sibling.to_string(),
                                                            input,
                                                        );
                                                    }
                                                } else {
                                                    tracing::debug!(
                                                        "Unhandled input: {}",
                                                        next_sibling
                                                    );
                                                }
                                            }
                                            _ => {
                                                tracing::debug!(
                                                    "Unhandled input: {}",
                                                    next_sibling
                                                );
                                            }
                                        }
                                    } else {
                                        tracing::debug!("Unhandled input: {}", next_sibling);
                                        if let Some(nested_attr) = child
                                            .as_node()
                                            .unwrap()
                                            .parent()
                                            .unwrap()
                                            .next_sibling()
                                        {
                                            tracing::debug!("Nested input: {}", nested_attr);
                                            for attr in nested_attr.children() {
                                                tracing::debug!("Nested input attr: {}", attr);
                                                for binding in attr.children() {
                                                    if binding.to_string() == "url" {
                                                        let url = binding.next_sibling().unwrap();
                                                        tracing::debug!(
                                                            "This is an url: {} - {}",
                                                            next_sibling,
                                                            url
                                                        );
                                                        let mut input =
                                                            Input::new(next_sibling.to_string());
                                                        input.url = url.to_string();
                                                        self.inputs.insert(
                                                            next_sibling.to_string(),
                                                            input,
                                                        );
                                                    }
                                                    tracing::debug!(
                                                        "Nested input attr binding: {}",
                                                        binding
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                                SyntaxKind::NODE_ATTR_SET => {}
                                _ => {
                                    tracing::debug!(
                                        "Unhandled input kind: {:?}",
                                        next_sibling.kind()
                                    );
                                    tracing::debug!("Unhandled input: {}", next_sibling);
                                }
                            }
                        }
                    }
                    if let Some(parent) = child.parent() {
                        tracing::debug!("Childs Parent: {}", parent);
                        tracing::debug!("Childs Parent: {}", parent);
                        if let Some(sibling) = parent.next_sibling() {
                            tracing::debug!("Childs Sibling: {}", sibling);
                        }
                    }
                }
                _ => {
                    tracing::debug!("UNMATCHED KIND: {:?}", child.kind());
                    tracing::debug!("UNMATCHED PATH: {}", child);
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
        tracing::debug!("\nInput: {node}\n");
        for (i, child) in node.children().enumerate() {
            tracing::debug!("Kind #:{i} {:?}", child.kind());
            if child.kind() == SyntaxKind::NODE_ATTRPATH {
                for attr in child.children() {
                    tracing::debug!("Child of ATTRPATH #:{i} {}", child);
                    tracing::debug!("Child of ATTR #:{i} {}", attr);
                    if attr.to_string() == "url" {
                        if let Some(prev_id) = attr.prev_sibling() {
                            if let Some(change) = self.changes.first() {
                                if self.commit {
                                    if let Change::Remove { id } = change {
                                        if *id == prev_id.to_string() {
                                            tracing::debug!("Removing: {id}");
                                            let empty = Root::parse("").syntax();
                                            return Some(empty);
                                        }
                                    }
                                }
                            }
                            if let Some(sibling) = child.next_sibling() {
                                tracing::debug!("This is an url from {} - {}", prev_id, sibling);
                                let mut input = Input::new(prev_id.to_string());
                                input.url = sibling.to_string();
                                self.inputs.insert(prev_id.to_string(), input);
                            }
                        }
                        tracing::debug!("This is the parent: {}", child.parent().unwrap());
                        tracing::debug!(
                            "This is the next_sibling: {}",
                            child.next_sibling().unwrap()
                        );
                        if let Some(parent) = child.parent() {
                            if let Some(sibling) = parent.next_sibling() {
                                tracing::debug!("This is an url:{} {}", attr, sibling);
                            }
                        }
                    }
                }
            }
            if child.kind() == SyntaxKind::NODE_ATTR_SET {
                for attr in child.children() {
                    tracing::debug!("Child of ATTRSET KIND #:{i} {:?}", child.kind());
                    tracing::debug!("Child of ATTRSET #:{i} {}", child);
                    for leaf in attr.children() {
                        if leaf.to_string() == "url" {
                            let id = child.prev_sibling().unwrap();
                            let uri = leaf.next_sibling().unwrap();
                            tracing::debug!("This is an url from {} - {}", id, uri,);
                            let mut input = Input::new(id.to_string());
                            input.url = uri.to_string();
                            self.inputs.insert(id.to_string(), input);

                            // Remove matched node.
                            if let Some(change) = self.changes.first() {
                                if self.commit {
                                    if let Change::Remove { id: candidate } = change {
                                        if *candidate == id.to_string() {
                                            tracing::debug!("Removing: {id}");
                                            let empty = Root::parse("").syntax();
                                            return Some(empty);
                                        }
                                    }
                                }
                            }
                        }
                        tracing::debug!("Child of ATTRSET KIND #:{i} {:?}", leaf.kind());
                        tracing::debug!("Child of ATTRSET CHILD #:{i} {}", leaf);
                    }
                }
            }
            tracing::debug!("Child #:{i} {}", child);
        }
        None
    }

    // pub fn walk_input_expr(&mut self, entry: Option<Expr>) -> Option<SyntaxNode> {
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
    //         // self.walk_input(Entry::AttrpathValue(attr_value));
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
