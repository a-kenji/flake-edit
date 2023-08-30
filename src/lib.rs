use std::{env, error::Error, fs};

use rnix::{
    ast::{
        AttrSet,
        Entry::{self, AttrpathValue},
        HasEntry,
    },
    parser::ParseError,
    tokenizer::Tokenizer,
    SyntaxKind, SyntaxNode,
};
use rowan::GreenNode;

// TODO:
// - parse out inputs
// - SyntaxKind(44) [inputs]
// - parse follows attribute and attrset outof the -> SyntaxKind(76) [attrset]
//
// NODE_STRING 63,
// NODE_IDENT 58,
// TOKEN_IDENT 44,
// TOKEN_DOT 21,
// NODE_ATTR_SET 76,
// NODE_ATTRPATH 55,
// TOKEN_URI 49,

#[derive(Debug, Clone)]
struct State {
    node: SyntaxNode,
    inputs: Vec<Input>,
}

impl State {
    fn change_input(&mut self) {
        todo!();
    }
    fn get_node(&self, input: &str) -> Option<Input> {
        self.inputs
            .clone()
            .into_iter()
            .filter(|n| n.name.as_str() == input)
            .next()
    }
    fn print_nodes(&self) {
        for c in self.node.children_with_tokens() {
            println!("Node: {c}");
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Input {
    name: String,
    attr_path: String,
    value: String,
    url: String,
    follows: Vec<String>,
}

impl Input {
    fn new(name: String, attr_path: String, value: String) -> Self {
        Self {
            name,
            attr_path,
            value,
            ..Self::default()
        }
    }
}

// TODO: impl TryFrom
impl From<Entry> for Input {
    fn from(entry: Entry) -> Self {
        if let AttrpathValue(attrpath_value) = entry {
            let value = attrpath_value.value().unwrap().to_string();
            let attr_path = attrpath_value.attrpath().unwrap().to_string();
            Self::new(attr_path.clone(), attr_path, value)
        } else {
            Self::default()
        }
        //     if let Some(value) = node.value() {
        //         return Self::new(value, value, value);
        //         // println!("attrpath: {attrpath}");
        //         // println!("value: {value}");
        //     }
    }
}

pub fn write_node(node: &SyntaxNode) -> SyntaxNode {
    todo!();
}

/// parse the input AST
pub fn parse_inputs(input: &GreenNode) -> Result<Vec<Input>, ParseError> {
    for node_or_token in input.children() {
        // If the SyntaxKind - 58 is inputs, then look ahead which attribute
        // match the attribute either with TOKEN_DOT or NODE_ATTR_SET
        // The SyntaxKind - 55 - ATTRPATH either needs to be parsed again, or is the final
        // assignment.
        if let Some(node) = node_or_token.as_node() {
            for node in node.children() {
                println!("{}", node);
                println!("Kind: {:?}\n", node.kind());
                if node.kind() == rowan::SyntaxKind(55) {
                    for child in node.as_node().unwrap().children() {
                        println!("Child Node of Attrset: \n");
                        println!("{}", child);
                        println!("Kind: {:?}\n", child.kind());
                    }
                }
            }
        }
    }
    Ok(vec![])
}

/// Parse the toplevel AST
pub fn parse_content(content: &str) -> Result<Vec<Input>, ParseError> {
    let (node, _errors) = rnix::parser::parse(Tokenizer::new(content));
    let mut is_input = false;
    let mut inputs = vec![];
    for c in node.children() {
        if let Some(node) = c.as_node() {
            for c in node.children() {
                if let Some(node) = c.as_node() {
                    for c in node.children() {
                        if let Some(node) = c.as_node() {
                            match c.kind() {
                                rowan::SyntaxKind(58) => {
                                    // println!(" Token - 58: {:?}", c.as_token());
                                }
                                rowan::SyntaxKind(55) => {
                                    for c in node.children() {
                                        if let Some(node) = c.as_node() {
                                            if is_input {
                                                inputs.push(node.to_string());
                                            }
                                            if c.to_string() == "inputs" {
                                                is_input = true;
                                                // if let rowan::SyntaxKind(58) = node.kind() {
                                                // println!("Inputs: ");
                                                // println!(" Node {:?}", node.children().next());
                                                // }
                                            }
                                        }
                                    }
                                    is_input = false;
                                }

                                _ => {}
                            }
                        }
                    }
                }
            }
        }
        if let Some(token) = c.as_token() {
            println!(" Toplevel Token: {:?}\n", token);
        }
    }
    // let set = match expr {
    //     ast::Expr::AttrSet(set) => set,
    //     _ => todo!(),
    //     // _ => return Err("root isn't a set".into()),
    // };
    // let inputs = input_values(set)?;
    // println!("Inputs: {:#?}", inputs);
    println!("Inputs: {:?}", inputs);
    // Ok(inputs)
    Ok(vec![])
}

fn input_values(set: AttrSet) -> Result<Vec<Input>, ParseError> {
    let mut res = Vec::new();
    for entry in set.entries() {
        if let AttrpathValue(attrpath_value) = &entry {
            if let Some(attrpath) = attrpath_value.attrpath() {
                if attrpath.to_string().starts_with("inputs") {
                    res.push(entry.into());
                }
            }
        }
    }
    Ok(res)
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
  };

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
    fn medium_flake() -> &'static str {
        todo!();
    }
    fn codepoint_flake() -> &'static str {
        r#"
        {
  description = "A slightly annoying flake";

  ${''
  inputs''} = rec {
    ${(((((''
    nixpkgs'')))))} = { url = "path:foo"; };

    "foo\nbar".url = "path:foo";
    "onlyone$$".url = path:foo;
  };

  outputs = { self, nixpkgs, ... }: {
    foo = 42;
  };
}
        "#
    }
    fn annoying_flake() -> &'static str {
        r#"
        {
  description = "A slightly annoying flake";

  ${''
  inputs''} = rec {
    ${(((((''
    nixpkgs'')))))} = { url = "path:foo"; };

    "foo\nbar"= { url = "path:foo"; follows = "nixpkgs"; };
    "onlyone$$".url = path:foo;

    notaflake = (({
      url = (path:foo);
      flake = (false);
      inputs = (({}));
    }));

    withtype = {
      type = "path";
      path = "/tmp/annoying/foo";
      rev = "da39a3ee5e6b4b0d3255bfef95601890afd80709";
    };

    indirect = {
      type = "indirect";
      id = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, ... }: {
    foo = 42;
  };
}
        "#
    }

    // #[test]
    // fn minimal_flake_parsed_ok() {
    //     parse_content(minimal_flake()).unwrap();
    // }
    // #[test]
    // fn minimal_flake_inputs_attrs_parsed_ok() {
    //     parse_content(minimal_flake_inputs_attrs()).unwrap();
    // }
    // #[test]
    // fn minimal_flake_inputs_correct_number() {
    //     assert_eq!(parse_content(minimal_flake()).unwrap().len(), 3);
    // }
    // #[test]
    // fn minimal_flake_inputs_attrs_correct_number() {
    //     assert_eq!(
    //         parse_content(minimal_flake_inputs_attrs()).unwrap().len(),
    //         3
    //     );
    // }
    // #[test]
    // fn minimal_flake_inputs_correct() {
    //     let mut expected = vec![];
    //     expected.push(Input::new(
    //         "inputs.nixpkgs.url".into(),
    //         "inputs.nixpkgs.url".into(),
    //         "github:nixos/nixpkgs/nixos-unstable".into(),
    //     ));
    //     expected.push(Input::new(
    //         "inputs.rust-overlay.url".into(),
    //         "inputs.rust-overlay.url".into(),
    //         "github:oxalica/rust-overlay".into(),
    //     ));
    //     expected.push(Input::new(
    //         "inputs.crane.url".into(),
    //         "inputs.crane.url".into(),
    //         "github:ipetkov/crane".into(),
    //     ));
    //     assert_eq!(parse_content(minimal_flake()).unwrap(), expected);
    // }
    // #[test]
    // fn no_inputs_flake_parsed_ok() {
    //     parse_content(no_inputs_flake()).unwrap();
    // }
    // #[test]
    // fn no_inputs_flake_inputs_correct_number() {
    //     assert_eq!(parse_content(no_inputs_flake()).unwrap().len(), 0);
    // }
    // #[test]
    // fn no_inputs_flake_inputs_correct() {
    //     let expected = vec![];
    //     assert_eq!(parse_content(no_inputs_flake()).unwrap(), expected);
    // }
    #[test]
    fn parse_simple_inputs() {
        let inputs = "inputs.nixpkgs.url = github:nixos/nixpkgs;";
        let (node, _errors) = rnix::parser::parse(Tokenizer::new(inputs));
        let expected = vec![Input::default()];
        assert_eq!(parse_inputs(&node).unwrap(), expected);
    }
    #[test]
    fn parse_simple_inputs_set() {
        let inputs = "inputs = { nixpkgs.url = github:nixos/nixpkgs }; ;";
        let (node, _errors) = rnix::parser::parse(Tokenizer::new(inputs));
        let expected = vec![Input::default()];
        assert_eq!(parse_inputs(&node).unwrap(), expected);
    }
    #[test]
    fn parse_simple_inputs_multiple() {
        let inputs = "inputs.nixpkgs.url = github:nixos/nixpkgs; inputs.crane.url = github:nix-community/crane;";
        let (node, _errors) = rnix::parser::parse(Tokenizer::new(inputs));
        let expected = vec![Input::default()];
        assert_eq!(parse_inputs(&node).unwrap(), expected);
    }
    #[test]
    fn parse_simple_inputs_single_flake_false() {
        let inputs = "inputs.nixpkgs.url = github:nixos/nixpkgs; inputs.nixpkgs.flake = false;";
        let (node, _errors) = rnix::parser::parse(Tokenizer::new(inputs));
        let expected = vec![Input::default()];
        assert_eq!(parse_inputs(&node).unwrap(), expected);
    }
    // #[test]
    // fn only_inputs_parsed_ok() {
    //     parse_content(only_inputs_flake()).unwrap();
    // }
    // #[test]
    // fn no_inputs_parsed_ok() {
    //     parse_content(no_inputs_flake()).unwrap();
    // }
    // #[test]
    // fn codepoint_flake_parsed_ok() {
    //     parse_content(codepoint_flake()).unwrap();
    // }
    // #[test]
    // fn codepoint_flake_parse_inputs() {
    //     parse_content(codepoint_flake());
    // }
    // #[test]
    // fn annoying_flake_parse_ok() {
    //     parse_content(codepoint_flake()).unwrap();
    // }
}

/// Interpret escape sequences in the nix string and return the converted value
/// TODO: escape tlipners tests propely
pub fn unescape(input: &str, multiline: bool) -> String {
    let mut output = String::new();
    let mut input = input.chars().peekable();
    loop {
        match input.next() {
            None => break,
            Some('"') if !multiline => break,
            Some('\\') if !multiline => match input.next() {
                None => break,
                Some('n') => output.push('\n'),
                Some('r') => output.push('\r'),
                Some('t') => output.push('\t'),
                Some(c) => output.push(c),
            },
            Some('\'') if multiline => match input.next() {
                None => {
                    output.push('\'');
                }
                Some('\'') => match input.peek() {
                    Some('\'') => {
                        input.next().unwrap();
                        output.push_str("''");
                    }
                    Some('$') => {
                        input.next().unwrap();
                        output.push('$');
                    }
                    Some('\\') => {
                        input.next().unwrap();
                        match input.next() {
                            None => break,
                            Some('n') => output.push('\n'),
                            Some('r') => output.push('\r'),
                            Some('t') => output.push('\t'),
                            Some(c) => output.push(c),
                        }
                    }
                    _ => break,
                },
                Some(c) => {
                    output.push('\'');
                    output.push(c);
                }
            },
            Some(c) => output.push(c),
        }
    }
    output
}
