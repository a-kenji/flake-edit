use std::{
    collections::{HashMap, HashSet},
    env,
    error::Error,
    fs,
};

use nix_uri::FlakeRef;
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
use rowan::{GreenNode, GreenToken, NodeOrToken};

// TODO:
// - parse out inputs
// - SyntaxKind(44) [inputs]
// - parse follows attribute and attrset outof the -> SyntaxKind(76) [attrset]
//
// NODE_STRING 63,
// NODE_IDENT 58,
// TOKEN_IDENT 44,
// TOKEN_DOT 21,
// NODE_ROOT 75,
// NODE_ATTR_SET 76,
// NODE_ATTRPATH 55,
// TOKEN_URI 49,

#[derive(Debug, Default, Clone)]
pub struct State {
    // All the parsed inputs that are present in the attr set
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
    // Traverses the whole flake.nix toplevel attr set.
    pub fn walk_attr_set(&mut self, node: &GreenNode) {
        let inputs = parse_inputs(node);
        if let Ok(inputs) = inputs {
            self.inputs = inputs;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Hash, Eq)]
pub struct Input {
    pub name: String,
    pub flake: bool,
    pub url: String,
    follows: Vec<String>,
}

impl Default for Input {
    fn default() -> Self {
        Self {
            name: String::new(),
            flake: true,
            url: String::new(),
            follows: vec![],
        }
    }
}

impl Input {
    fn new(name: String) -> Self {
        Self {
            name,
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
            Self::new(attr_path)
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
    let mut res: Vec<Input> = vec![];
    let other = input.clone();
    println!("Original: {}", input);
    // SyntaxKind 75 - NODE_ROOT
    println!("Original Kind: {:?}\n", input.kind());
    let rinput = SyntaxNode::new_root(input.clone());
    for walk_node_or_token in rinput.preorder_with_tokens() {
        match walk_node_or_token {
            rowan::WalkEvent::Enter(node_or_token) => {
                match &node_or_token {
                    NodeOrToken::Node(node) => {
                        match node.kind() {
                            SyntaxKind::TOKEN_URI => {}
                            // TODO: PushDown Automata with recursive attrpaths
                            SyntaxKind::NODE_IDENT => {}
                            SyntaxKind::NODE_STRING => {}
                            SyntaxKind::TOKEN_IDENT => {
                                println!("Token Ident: {}", node);
                            }
                            SyntaxKind::TOKEN_STRING_CONTENT => {
                                if let Some(token) = node_or_token.as_token() {
                                    println!("{token}");
                                    if let Ok(mut flake_ref) = FlakeRef::from(token.to_string()) {
                                        flake_ref.params.set_dir(Some("assets".to_owned()));
                                        let replacement_token = GreenToken::new(
                                            rowan::SyntaxKind(50),
                                            &flake_ref.to_string(),
                                        );
                                        let tree = token.replace_with(replacement_token);
                                        println!("Tree: {}", tree);
                                    }
                                }
                            }
                            // Skip unneccessary Token
                            SyntaxKind::TOKEN_WHITESPACE
                            | SyntaxKind::TOKEN_R_BRACE
                            | SyntaxKind::TOKEN_L_BRACE
                            | SyntaxKind::TOKEN_SEMICOLON => {
                                continue;
                            }
                            // Print Select Token
                            SyntaxKind::NODE_ATTR_SET
                            | SyntaxKind::NODE_ATTRPATH
                            // | SyntaxKind::NODE_ATTRPATH_VALUE
                                => {
                                let new_root = SyntaxNode::new_root(node.green().into());
                                println!("Create new root: {new_root:?}");
                                for walk_node_or_token in new_root.preorder_with_tokens() {
                                    match walk_node_or_token {
                                        rowan::WalkEvent::Enter(node_or_token) => {
                                            match &node_or_token {
                                                NodeOrToken::Node(node) => {
                                                    match node.kind() {
                                                        SyntaxKind::NODE_ATTRPATH => {
                                                            if node.to_string() == "description" {
                                                                println!(
                                                                    "Description Node: {node}"
                                                                );
                                                                print_node_enter_info(
                                                                    &node_or_token,
                                                                );
                                                                continue;
                                                            }
                                                            if node.to_string() == "inputs" {
                                                                println!("Input Node: {node}");
                                                                print_node_enter_info(
                                                                    &node_or_token,
                                                                );
                                                                for node in node.children() {
                                                                    println!(
                                                            "Input NODE_ATTRPATH NODE Children: {node}"
                                                        );
                                                                }
                                                                for node in node.siblings(
                                                                    rowan::Direction::Next,
                                                                ) {
                                                                    println!(
                                                            "Input NODE_ATTRPATH NODE Siblings: {node}"
                                                        );
                                                                    println!(
                                                            "Input NODE_ATTRPATH NODE Sibling Kind: {:?}", node.kind()
                                                        );
                                                                    if node.kind()
                                                                        == SyntaxKind::NODE_ATTR_SET
                                                                    {
                                                                        println!(
                                                                            "Matched node: {node}"
                                                                        );
                                                                        let inputs =
                                                                        inputs_from_node_attr_set(
                                                                            node.green().into(),
                                                                        );
                                                                        println!(
                                                                            "Extending with: {:?}",
                                                                            inputs
                                                                        );
                                                                        res.extend(inputs);
                                                                    }
                                                                }
                                                            }
                                                            // print_node_enter_info(&node);
                                                        }
                                                        _ => {
                                                            // print_node_enter_info(&node);
                                                        }
                                                    }
                                                }
                                                NodeOrToken::Token(_) => {}
                                            }
                                        }
                                        rowan::WalkEvent::Leave(_node) => {} // print_node_leave_info(&node),
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    NodeOrToken::Token(_) => {}
                }
            }
            rowan::WalkEvent::Leave(node) => match node.kind() {
                SyntaxKind::TOKEN_COMMENT
                | SyntaxKind::TOKEN_ERROR
                | SyntaxKind::TOKEN_WHITESPACE
                | SyntaxKind::TOKEN_L_BRACE
                | SyntaxKind::TOKEN_R_BRACE
                | SyntaxKind::TOKEN_L_BRACK
                | SyntaxKind::TOKEN_R_BRACK
                | SyntaxKind::TOKEN_COLON
                | SyntaxKind::TOKEN_COMMA
                | SyntaxKind::TOKEN_SEMICOLON
                | SyntaxKind::TOKEN_DOT
                | SyntaxKind::TOKEN_L_PAREN
                | SyntaxKind::TOKEN_R_PAREN => {
                    continue;
                }
                _ => {}
            },
        }
    }
    println!("Original: {}", input);
    println!("Changed: {}", input);
    Ok(res)
}

// Handles attrsets of the following form they are assumed to be nested inside the inputs attribute:
// { nixpkgs.url = "github:nixos/nixpkgs"; crane.url = "github:nix-community/crane"; }
// { nixpkgs.url = "github:nixos/nixpkgs";}
fn inputs_from_node_attr_set(node: GreenNode) -> Vec<Input> {
    let node = SyntaxNode::new_root(node);
    let mut res = vec![];
    for node_walker in node.preorder_with_tokens() {
        match node_walker {
            rowan::WalkEvent::Enter(node_or_token) => {
                println!("Inputs from node attrs set");
                print_node_enter_info(&node_or_token);
                if let Some(node) = node_or_token.as_node() {
                    if SyntaxKind::NODE_ATTRPATH_VALUE == node.kind() {
                        if let Some(input) = input_from_node_attrpath_value(node) {
                            res.push(input);
                        }
                    }
                }
            }
            rowan::WalkEvent::Leave(_) => {}
        }
    }
    res
}

// Handles NODE_ATTRPATH_VALUES for a single input
// Example: crane.url = "github:nix-community/crane";
// TODO: handle nested attribute sets:
// Example: crane = { url = "github:nix-community/crane";};
fn input_from_node_attrpath_value(node: &SyntaxNode) -> Option<Input> {
    println!();
    println!("ATTRPATHVALUE:");
    println!("{node}");
    let mut res: Option<Input> = None;
    for walker in node.preorder_with_tokens() {
        match walker {
            rowan::WalkEvent::Enter(node_or_token) => match &node_or_token {
                NodeOrToken::Node(node) => {
                    match node.kind() {
                        SyntaxKind::NODE_ATTRPATH => {}
                        SyntaxKind::NODE_IDENT => {
                            if res.is_none() {
                                res = Some(Input::new(node.to_string()));
                            }
                        }
                        // TODO: preserve string vs literal
                        SyntaxKind::NODE_STRING | SyntaxKind::NODE_LITERAL => {
                            if let Some(ref mut input) = res {
                                input.url = node.to_string();
                                return res;
                            }
                        }
                        _ => {}
                    }
                    println!("Node: {node}");
                    println!("Kind: {:?}", node.kind());
                }
                NodeOrToken::Token(token) => {
                    println!("Token: {token}");
                    println!("Token Kind: {:?}", token.kind());
                }
            },
            rowan::WalkEvent::Leave(_) => {}
        }
    }
    None
}

pub fn print_node_enter_info(node: &NodeOrToken<rnix::SyntaxNode, rnix::SyntaxToken>) {
    println!("Enter: {node}");
    println!("Enter Kind: {:?}", node.kind());
    println!("Enter Parent: {:?}", node.parent());
    if let Some(parent) = node.parent() {
        println!("Enter Parent Node: {:?}", parent);
        println!("Enter Parent Node Kind: {:?}", parent.kind());
    }
    if let Some(node) = node.as_node() {
        println!("Enter Green Kind: {:?}", node.green().kind());
        for child in node.children() {
            println!("Enter Children: {:?}", child);
            println!("Enter Children Kind: {:?}", child.green().kind());
        }
        println!("Node Next Sibling: {:?}", node.next_sibling());
        println!("Node Prev Sibling: {:?}", node.prev_sibling());
    }
    if let Some(token) = node.as_token() {
        println!("Token: {}", token);
    }
    // if let Some(kind) = node.as_node() {
    //     println!("Enter Node Kind: {:?}", kind);
    // }
    // if let Some(kind) = node.as_token() {
    //     println!("Enter Token Kind: {:?}", kind);
    // }
    println!("Node Index: {}", node.index());
    println!();
}

pub fn print_node_leave_info(node: &NodeOrToken<rnix::SyntaxNode, rnix::SyntaxToken>) {
    println!("Leave: {node}");
    println!("Leave Index: {:?}", node.index());
    println!("Leave Kind: {:?}", node.kind());
    if let Some(node) = node.as_node() {
        println!("Leave Green Kind: {:?}", node.green().kind());
        println!("Leave Kind Next Sibling: {:?}", node.next_sibling());
        println!("Leave Kind Prev Sibling: {:?}", node.prev_sibling());
    }
    println!("Leave Kind Parent: {:?}", node.parent());
    println!();
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
        let inputs = r#"{ inputs.nixpkgs.url = "github:nixos/nixpkgs";}"#;
        let (node, _errors) = rnix::parser::parse(Tokenizer::new(inputs));
        let expected = vec![Input::default()];
        assert_eq!(parse_inputs(&node).unwrap(), expected);
    }
    #[test]
    fn parse_simple_inputs_alt() {
        let inputs = r#"{ inputs = { nixpkgs.url = "github:nixos/nixpkgs";};}"#;
        let (node, _errors) = rnix::parser::parse(Tokenizer::new(inputs));
        let expected = vec![Input::default()];
        assert_eq!(parse_inputs(&node).unwrap(), expected);
    }
    // #[test]
    // fn parse_simple_inputs_description() {
    //     let inputs =
    //         r#"{ description = "This is a text."; inputs.nixpkgs.url = "github:nixos/nixpkgs";}"#;
    //     let (node, _errors) = rnix::parser::parse(Tokenizer::new(inputs));
    //     let expected = vec![Input::default()];
    //     assert_eq!(parse_inputs(&node).unwrap(), expected);
    // }
    // #[test]
    // fn parse_simple_inputs_set() {
    //     let inputs = r#"{inputs = { nixpkgs.url = "github:nixos/nixpkgs"; };}"#;
    //     let (node, _errors) = rnix::parser::parse(Tokenizer::new(inputs));
    //     let expected = vec![Input::default()];
    //     assert_eq!(parse_inputs(&node).unwrap(), expected);
    // }
    // #[test]
    // fn parse_simple_inputs_set_description() {
    //     let inputs = r#"{description = "This is a text."; inputs = { nixpkgs.url = "github:nixos/nixpkgs"; };}"#;
    //     let (node, _errors) = rnix::parser::parse(Tokenizer::new(inputs));
    //     let expected = vec![Input::default()];
    //     assert_eq!(parse_inputs(&node).unwrap(), expected);
    // }
    #[test]
    fn parse_simple_inputs_set_multiple() {
        let inputs = r#"{inputs = { nixpkgs.url = "github:nixos/nixpkgs"; crane.url = "github:nix-community/crane"; };}"#;
        let (node, _errors) = rnix::parser::parse(Tokenizer::new(inputs));
        let expected = vec![Input::default()];
        assert_eq!(parse_inputs(&node).unwrap(), expected);
    }
    // #[test]
    // fn parse_simple_inputs_set_multiple_no_flake() {
    //     let inputs = r#"{inputs = { nixpkgs.url = "github:nixos/nixpkgs"; crane.url = "github:nix-community/crane"; crane.flake = false; };}"#;
    //     let (node, _errors) = rnix::parser::parse(Tokenizer::new(inputs));
    //     let expected = vec![Input::default()];
    //     assert_eq!(parse_inputs(&node).unwrap(), expected);
    // }
    // #[test]
    // fn parse_simple_inputs_set_multiple_no_flake_description() {
    //     let inputs = r#"{description = "This is a text."; inputs = { nixpkgs.url = "github:nixos/nixpkgs"; crane.url = "github:nix-community/crane"; crane.flake = false; };}"#;
    //     let (node, _errors) = rnix::parser::parse(Tokenizer::new(inputs));
    //     let expected = vec![Input::default()];
    //     assert_eq!(parse_inputs(&node).unwrap(), expected);
    // }
    // #[test]
    // fn parse_simple_inputs_set_multiple_no_flake_together() {
    //     let inputs = r#"{inputs = { nixpkgs.url = "github:nixos/nixpkgs"; crane = { url = "github:nix-community/crane"; flake = false; };};}"#;
    //     let (node, _errors) = rnix::parser::parse(Tokenizer::new(inputs));
    //     let expected = vec![Input::default()];
    //     assert_eq!(parse_inputs(&node).unwrap(), expected);
    // }
    // #[test]
    // fn parse_simple_inputs_multiple() {
    //     let inputs = "{inputs.nixpkgs.url = github:nixos/nixpkgs; inputs.crane.url = github:nix-community/crane;}";
    //     let (node, _errors) = rnix::parser::parse(Tokenizer::new(inputs));
    //     println!("{:?}", _errors);
    //     let expected = vec![Input::default()];
    //     assert_eq!(parse_inputs(&node).unwrap(), expected);
    // }
    // #[test]
    // fn parse_simple_inputs_multiple_description() {
    //     let inputs = r#"{description = "This is a Text"; inputs.nixpkgs.url = "github:nixos/nixpkgs"; inputs.crane.url = "github:nix-community/crane";}"#;
    //     let (node, _errors) = rnix::parser::parse(Tokenizer::new(inputs));
    //     println!("{:?}", _errors);
    //     let expected = vec![Input::default()];
    //     assert_eq!(parse_inputs(&node).unwrap(), expected);
    // }
    // #[test]
    // fn parse_simple_inputs_single_flake_false() {
    //     let inputs = "inputs.nixpkgs.url = github:nixos/nixpkgs; inputs.nixpkgs.flake = false;";
    //     let (node, _errors) = rnix::parser::parse(Tokenizer::new(inputs));
    //     let expected = vec![Input::default()];
    //     assert_eq!(parse_inputs(&node).unwrap(), expected);
    // }
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
