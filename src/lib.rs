pub mod diff;
pub mod error;
mod git;
mod input;
pub mod walk;

use std::collections::HashMap;

use nix_uri::FlakeRef;
use rnix::{
    ast::{
        Entry::{self, AttrpathValue},
        Expr, HasEntry,
    },
    parser::ParseError,
    tokenizer::Tokenizer,
    SyntaxKind, SyntaxNode,
};
use rowan::{GreenNode, GreenToken, NodeOrToken};

use self::input::Input;

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
    pub inputs: HashMap<String, Input>,
    changes: Vec<Change>,
}

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub enum Change {
    #[default]
    None,
    Add {
        id: Option<String>,
        uri: Option<String>,
    },
    Remove {
        id: String,
    },
    Pin {
        id: String,
    },
    Change {
        id: Option<String>,
        ref_or_rev: Option<String>,
    },
}

impl Change {
    pub fn id(&self) -> Option<String> {
        match self {
            Change::None => None,
            Change::Add { id, .. } => id.clone(),
            Change::Remove { id } => Some(id.clone()),
            Change::Change { id, .. } => id.clone(),
            Change::Pin { id } => Some(id.clone()),
        }
    }
    pub fn is_remove(&self) -> bool {
        match self {
            Change::Remove { .. } => true,
            _ => false,
        }
    }
}

impl State {
    pub fn add_change(&mut self, change: Change) {
        self.changes.push(change);
    }
    fn find_change(&self, target_id: String) -> Option<Change> {
        for change in &self.changes {
            match change {
                Change::None => {}
                Change::Remove { id } | Change::Pin { id } => {
                    if *id == target_id {
                        return Some(change.clone());
                    }
                }
                Change::Add { id, .. } | Change::Change { id, .. } => {
                    if let Some(id) = id {
                        if *id == target_id {
                            return Some(change.clone());
                        }
                    }
                }
            }
        }
        None
    }
    pub fn add_input(&mut self, key: &str, input: Input) {
        self.inputs.insert(key.into(), input);
    }
    pub fn add_follows(&mut self, key: &str, follows: input::Follows) {
        if let Some(input) = self.inputs.get_mut(key) {
            input.follows.push(follows);
        }
    }

    // Traverses the whole flake.nix toplevel attr set.
    pub fn walk_attr_set(&mut self, node: &GreenNode) {
        let _ = self.parse_inputs(node);
    }
    // Traverses the whole flake.nix toplevel attr set.
    pub fn walk_expr_set(&mut self, stream: &str) {
        let root = rnix::Root::parse(stream).ok().unwrap();

        let expr = root.expr().unwrap();

        let attr_set = match expr {
            Expr::AttrSet(attr_set) => Some(attr_set),
            _ => None,
        }
        .unwrap();

        for attr in attr_set.attrpath_values() {
            if let Some(path) = attr.attrpath() {
                match path.to_string().as_str() {
                    "inputs" => self.walk_inputs(attr.value()),
                    "description" | "outputs" => {}
                    _ => todo!("Root attribute incorrect."),
                }
            }
        }
    }

    fn walk_inputs(&mut self, attr: Option<Expr>) {
        let entry = match attr {
            Some(entry) => match entry {
                Expr::AttrSet(attr_set) => Some(attr_set),
                _ => {
                    println!("Not matched: {:?}", entry);
                    None
                }
            },
            None => todo!(),
        }
        .unwrap();

        for attrs in entry.attrpath_values() {
            let path = attrs.attrpath().unwrap();
            let value = attrs.value().unwrap();
            println!("Path: {}", path);
            for attr in path.attrs() {
                println!("Attr: {}", attr);
            }
            match &value {
                Expr::Str(uri) => {
                    println!("Uri: {uri}");
                }
                Expr::AttrSet(attr_set) => {
                    println!("AttrSet: {attr_set}");
                    // self.walk_inputs(Some(value.clone()));
                    self.walk_input_attrpath_values(Some(AttrpathValue(attrs)));
                }
                _ => todo!(),
            }
            // println!("Value: {}", value);
        }
    }
    fn walk_input_attrpath_values(&mut self, attrpath_values: Option<Entry>) {
        let attrpath_values = match attrpath_values {
            Some(attr) => match attr {
                Entry::Inherit(_) => None,
                AttrpathValue(attrpath_values) => Some(attrpath_values),
            },
            None => None,
        }
        .unwrap();

        let path = attrpath_values.attrpath().unwrap();
        let attrs = attrpath_values.value();

        let attrs = match attrs {
            Some(entry) => match entry {
                Expr::AttrSet(attr_set) => Some(attr_set),
                _ => {
                    println!("Not matched: {:?}", entry);
                    None
                }
            },
            None => todo!(),
        }
        .unwrap();

        println!("Path: {}", path);
        for attrs in attrs.attrpath_values() {
            let path = attrs.attrpath().unwrap();
            let value = attrs.value().unwrap();
            println!("Path: {}", path);
            for attr in path.attrs() {
                println!("Attr: {}", attr);
            }
            match &value {
                Expr::Str(uri) => {
                    println!("Uri: {uri}");
                }
                Expr::AttrSet(attr_set) => {
                    println!("AttrSet: {attr_set}");
                    // self.walk_inputs(Some(value.clone()));
                    self.walk_inputs(Some(value.clone()));
                }
                _ => todo!(),
            }
        }
    }
    /// parse the input AST
    pub fn parse_inputs(&mut self, input: &GreenNode) -> Result<(), ParseError> {
        let _other = input.clone();
        tracing::debug!("Original: {}", input);
        // SyntaxKind 75 - NODE_ROOT
        tracing::debug!("Original Kind: {:?}\n", input.kind());
        // TODO: test if node is root;
        let rinput = SyntaxNode::new_root(input.clone());
        for walk_node_or_token in rinput.preorder_with_tokens() {
            match walk_node_or_token {
                rowan::WalkEvent::Enter(node_or_token) => {
                    match &node_or_token {
                        NodeOrToken::Node(main_node) => {
                            tracing::debug!("Node: {main_node}");
                            tracing::debug!("Node Kind: {:?}", main_node.kind());
                            match main_node.kind() {
                                SyntaxKind::NODE_ATTR_SET
                                | SyntaxKind::NODE_ATTRPATH
                                | SyntaxKind::NODE_ATTRPATH_VALUE => {
                                    let new_root = SyntaxNode::new_root(main_node.green().into());
                                    tracing::debug!("Create new root: {new_root:?}");
                                    tracing::debug!("Create new root: {new_root}");
                                    for walk_node_or_token in new_root.preorder_with_tokens() {
                                        match walk_node_or_token {
                                            rowan::WalkEvent::Enter(node_or_token) => {
                                                match &node_or_token {
                                                    NodeOrToken::Node(node) => {
                                                        match node.kind() {
                                                            SyntaxKind::NODE_ATTRPATH => {
                                                                tracing::debug!(
                                                                    "Toplevel Node: {node}"
                                                                );
                                                                tracing::debug!(
                                                                    "Toplevel Node Kind: {:?}",
                                                                    node.kind()
                                                                );
                                                                if node.to_string() == "description"
                                                                {
                                                                    tracing::debug!(
                                                                        "Description Node: {node}"
                                                                    );
                                                                    continue;
                                                                }
                                                                if node.to_string() == "inputs"
                                                                // || node
                                                                //     .first_child()
                                                                //     .map(|c| {
                                                                //         c.to_string()
                                                                //             == "inputs"
                                                                //     })
                                                                //     .unwrap_or_default()
                                                                {
                                                                    tracing::debug!(
                                                                        "Input Node: {node}"
                                                                    );
                                                                    for node in node.children() {
                                                                        tracing::debug!(
                                                            "Input NODE_ATTRPATH NODE Children: {node}"
                                                        );
                                                                        tracing::debug!(
                                                            "Input NODE_ATTRPATH NODE Children index: {}", node.index()
                                                        );
                                                                    }
                                                                    for node in node.siblings(
                                                                        rowan::Direction::Next,
                                                                    ) {
                                                                        tracing::debug!(
                                                            "Input NODE_ATTRPATH NODE Siblings: {node}"
                                                        );
                                                                        tracing::debug!(
                                                            "Input NODE_ATTRPATH NODE Sibling Kind: {:?}", node.kind()
                                                        );
                                                                        tracing::debug!(
                                                            "Input NODE_ATTRPATH NODE Sibling Index: {:?}", node.index()
                                                        );
                                                                        match node.kind() {
                                                                                    SyntaxKind::NODE_ATTRPATH => {
                                                                                    }
                                                                                    SyntaxKind::NODE_ATTR_SET => {
                                                                                    println!("ATTRS_SET_NODE: {node}");
                                                                                    // Node that is
                                                                                    // constructed
                                                                                    // here needs
                                                                                    // to be a
                                                                                    // NODE_ATTR_SET

                                                                        tracing::info!(
                                                                            "Matched node: {node}"
                                                                        );
                                                                        if let Some(replacement) = self.inputs_from_node_attr_set(
                                                                            node.green().into(),
                                                                        ) {
                                                                                        let tree = node.replace_with(replacement);
                                                                                        // println!("{}", tree);
                                                                                        let whole_tree = main_node.replace_with(tree);
                                                                                        println!("Whole Tree:\n{}", whole_tree);
                                                                                    }
                                                                    }

                                                                                    _ => {}
                                                                                }
                                                                    }
                                                                } else {
                                                                    for child in node.children() {
                                                                        tracing::debug!(
                                                                            "Print Child: {}",
                                                                            child
                                                                        );
                                                                    }
                                                                    let child =
                                                                        node.first_child().unwrap();
                                                                    tracing::debug!(
                                                                        "First Child: {}",
                                                                        child
                                                                    );
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
                rowan::WalkEvent::Leave(_) => {}
            }
        }
        // println!("Original: {}", input);
        // println!("Changed: {}", input);
        Ok(())
    }
    /// Handles attrsets of the following form they are assumed to be nested inside the inputs attribute:
    /// { nixpkgs.url = "github:nixos/nixpkgs"; crane.url = "github:nix-community/crane"; }
    /// { nixpkgs.url = "github:nixos/nixpkgs";}
    /// TODO: create a GreenNode from all changed inputs
    fn inputs_from_node_attr_set(&mut self, node: GreenNode) -> Option<GreenNode> {
        tracing::debug!("Inputs from node attrs node: {node}");
        let root_node = SyntaxNode::new_root(node);
        // Only query root attributes in the toplevel
        let parent_id = 0;
        // let mut res = vec![];
        for node_walker in root_node.preorder_with_tokens() {
            match node_walker {
                rowan::WalkEvent::Enter(node_or_token) => {
                    tracing::debug!("Inputs from node attrs set");
                    if let Some(node) = node_or_token.as_node() {
                        if SyntaxKind::NODE_ATTRPATH_VALUE == node.kind() {
                            if let Some(parent) = node.parent() {
                                if parent.index() == parent_id {
                                    if let Some(replacement) =
                                        self.input_from_node_attrpath_value(node)
                                    {
                                        tracing::debug!("Original Node: {node}");
                                        tracing::debug!("Node Changed: {replacement}");
                                        tracing::debug!("Node Kind: {:?}", node.kind());
                                        tracing::debug!(
                                            "Node Green Kind: {:?}",
                                            node.green().kind()
                                        );
                                        tracing::debug!(
                                            "Replacement Kind: {:?}",
                                            replacement.kind()
                                        );
                                        let tree = root_node.replace_with(replacement);
                                        tracing::debug!("Changed tree:\n {}", tree);
                                        return Some(tree);
                                        // res.push(input);
                                        // self.add_input(input);
                                    }
                                }
                            }
                        }
                    }
                }
                rowan::WalkEvent::Leave(_) => {}
            }
        }
        None
    }
    // Handles NODE_ATTRPATH_VALUES for a single input
    // Example: crane.url = "github:nix-community/crane";
    // TODO: handle nested attribute sets:
    // Example: crane = { url = "github:nix-community/crane";};
    fn input_from_node_attrpath_value(&mut self, input_node: &SyntaxNode) -> Option<GreenNode> {
        tracing::debug!("ATTRPATHVALUE:");
        tracing::debug!("Input node: {input_node}");
        let mut res: Option<GreenNode> = None;
        let mut input: Option<Input> = None;
        let mut id: Option<String> = None;
        let mut follows: Option<input::FollowsBuilder> = None;
        for walker in input_node.preorder_with_tokens() {
            match walker {
                rowan::WalkEvent::Enter(node_or_token) => match &node_or_token {
                    NodeOrToken::Node(node) => {
                        println!("Node: {node}");
                        println!("Node Kind: {:?}", node.kind());
                        println!("Node ID: {:?}", node.index());
                        match node.kind() {
                            SyntaxKind::NODE_ATTRPATH => {}
                            SyntaxKind::NODE_ATTRPATH_VALUE => {
                                println!("ENTER: NODE_ATTRPATH_VALUE: \n{}", node);
                            }
                            SyntaxKind::NODE_IDENT => {
                                tracing::debug!("IDENT KIND: {:?}", node.kind());
                                tracing::debug!("IDENT: {}", node);
                                if id.is_some() && node.to_string() == "inputs" {
                                    // This is now a potential follows node
                                    follows = Some(input::FollowsBuilder::default());
                                }

                                if let Some(id) = &id {
                                    if let Some(ref mut builder) = follows {
                                        println!("Pushing: {:?}", node.to_string());
                                        if let Some(built) = builder.push_str(&node.to_string()) {
                                            println!("Follows: {:?}", built);
                                            self.add_follows(id, built);
                                            follows = None;
                                        }
                                    }
                                }

                                if input.is_none()
                                    && id.is_none()
                                    && node.to_string() != "url"
                                    && node.to_string() != "inputs"
                                {
                                    input = Some(Input::new(node.to_string()));
                                }

                                if id.is_none() {
                                    id = Some(node.to_string())
                                }
                            }
                            // TODO: preserve string vs literal
                            SyntaxKind::NODE_STRING
                            | SyntaxKind::NODE_LITERAL
                            | SyntaxKind::TOKEN_URI => {
                                if let Some(ref mut input) = input {
                                    let url =
                                        node.to_string().strip_prefix('\"').unwrap().to_string();
                                    let url =
                                        url.to_string().strip_suffix('\"').unwrap().to_string();
                                    input.url = url.clone();
                                    tracing::debug!("Adding input: {input:?}");
                                    self.add_input(&input.id, input.clone());

                                    let maybe_change = self.find_change(input.id.clone());
                                    if let Some(change) = maybe_change {
                                        tracing::debug!("Change: {change:?}");
                                        if let Ok(mut flake_ref) = FlakeRef::from(&url) {
                                            match change {
                                                Change::None => todo!(),
                                                Change::Add { .. } => todo!(),
                                                Change::Remove { .. } => todo!(),
                                                Change::Change { ref_or_rev, .. } => {
                                                    flake_ref
                                                        .r#type
                                                        .ref_or_rev(ref_or_rev)
                                                        .unwrap();
                                                    flake_ref
                                                        .params
                                                        .set_dir(Some("assets".to_owned()));
                                                }
                                                Change::Pin { id } => todo!(),
                                            }
                                            let replacement_node = GreenNode::new(
                                                rowan::SyntaxKind(63),
                                                std::iter::once(NodeOrToken::Token(
                                                    GreenToken::new(
                                                        rowan::SyntaxKind(63),
                                                        format!("\"{}\"", &flake_ref.to_string())
                                                            .as_str(),
                                                    ),
                                                )),
                                            );
                                            let tree = node.replace_with(replacement_node);
                                            println!("Tree: {}", tree);
                                            println!("Tree kind: {:?}", tree.kind());
                                            println!("Input Node kind: {:?}", input_node.kind());
                                            println!(
                                                "Input Node Green kind: {:?}",
                                                input_node.green().kind()
                                            );
                                            // let tree = GreenNode::new(
                                            //     rowan::SyntaxKind(77),
                                            //     std::iter::once(NodeOrToken::Node(tree)),
                                            // );
                                            res = Some(tree);
                                        }
                                    }
                                }
                                if input.is_some() {
                                    input = None;
                                }
                                if let Some(id) = &id {
                                    if let Some(ref mut builder) = follows {
                                        println!("Pushing: {:?}", node.to_string());
                                        if let Some(built) = builder.push_str(&node.to_string()) {
                                            println!("Follows: {:?}", built);
                                            self.add_follows(id, built);
                                            follows = None;
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                        tracing::debug!("Node: {node}");
                        tracing::debug!("Kind: {:?}", node.kind());
                    }
                    NodeOrToken::Token(token) => {
                        tracing::debug!("Token: {token}");
                        tracing::debug!("Token Kind: {:?}", token.kind());
                    }
                },
                rowan::WalkEvent::Leave(node_or_token) => match &node_or_token {
                    NodeOrToken::Node(_node) => match _node.kind() {
                        SyntaxKind::NODE_ATTRPATH_VALUE => {
                            println!("LEAVE: NODE_ATTRPATH_VALUE: \n{}", _node);
                        }

                        _ => {}
                    },
                    NodeOrToken::Token(_) => {}
                },
            }
        }
        res
    }
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
  description = "Thanks till.";

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
    // #[test]
    // fn parse_simple_inputs() {
    //     let inputs = r#"{ inputs.nixpkgs.url = "github:nixos/nixpkgs";}"#;
    //     let (node, _errors) = rnix::parser::parse(Tokenizer::new(inputs));
    //     let expected = vec![Input::default()];
    //     assert_eq!(parse_inputs(&node).unwrap(), expected);
    // }
    // #[test]
    // fn parse_simple_inputs_alt() {
    //     let inputs = r#"{ inputs = { nixpkgs.url = "github:nixos/nixpkgs";};}"#;
    //     let (node, _errors) = rnix::parser::parse(Tokenizer::new(inputs));
    //     let expected = vec![Input::default()];
    //     assert_eq!(parse_inputs(&node).unwrap(), expected);
    // }
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
    // #[test]
    // fn parse_simple_inputs_set_multiple() {
    //     let inputs = r#"{inputs = { nixpkgs.url = "github:nixos/nixpkgs"; crane.url = "github:nix-community/crane"; };}"#;
    //     let (node, _errors) = rnix::parser::parse(Tokenizer::new(inputs));
    //     let expected = vec![Input::default()];
    //     assert_eq!(parse_inputs(&node).unwrap(), expected);
    // }
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
    // fn setup_inputs(stream: &str) -> State {
    //     let (node, _errors) = rnix::parser::parse(Tokenizer::new(stream));
    //     let mut state = State::default();
    //     state.walk_attr_set(&node);
    //     state
    // }
    // #[test]
    // fn parse_simple_inputs_single_old_uri() {
    //     let inputs = "{ inputs.nixpkgs.url = github:nixos/nixpkgs;}";
    //     let state = setup_inputs(inputs);
    //     insta::assert_yaml_snapshot!(state.inputs);
    // }
    // #[test]
    // fn parse_simple_inputs_single_alt_old_uri() {
    //     let inputs = "{ inputs = { nixpkgs.url = github:nixos/nixpkgs;};}";
    //     let state = setup_inputs(inputs);
    //     insta::assert_yaml_snapshot!(state.inputs);
    // }
    // TODO
    // #[test]
    // fn parse_simple_inputs_single() {
    //     let inputs = r#"{ inputs.nixpkgs.url = "github:nixos/nixpkgs";}"#;
    //     let state = setup_inputs(inputs);
    //     insta::assert_yaml_snapshot!(state.inputs);
    // }
    //     let inputs = "{inputs.nixpkgs.url = github:nixos/nixpkgs; inputs.crane.url = github:nix-community/crane;}";
    // #[test]
    // fn parse_simple_input_two_urls() {
    //     let inputs = r#"{ inputs = { nixpkgs.url = "github:nixos/nixpkgs"; crane.url = "github:nix-community/crane";};}"#;
    //     let state = setup_inputs(inputs);
    //     insta::with_settings!({sort_maps => true}, {
    //         insta::assert_yaml_snapshot!(state.inputs);
    //     });
    // }
    // #[test]
    // fn parse_simple_input_url() {
    //     let inputs = r#"{ inputs = { nixpkgs.url = "github:nixos/nixpkgs";};}"#;
    //     let state = setup_inputs(inputs);
    //     insta::with_settings!({sort_maps => true}, {
    //         insta::assert_yaml_snapshot!(state.inputs);
    //     });
    // }
    // // #[test]
    // // fn parse_simple_input_url_alt() {
    // //     let inputs = r#"{ inputs.nixpkgs.url = "github:nixos/nixpkgs";}"#;
    // //     let state = setup_inputs(inputs);
    // //     insta::assert_yaml_snapshot!(state.inputs);
    // // }
    // #[test]
    // fn parse_single_follows() {
    //     let inputs = r#"{
    //           description = "Manage your flake inputs comfortably.";
    //
    //           inputs = {
    //             nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    //             rust-overlay = {
    //               url = "github:oxalica/rust-overlay";
    //               inputs.nixpkgs.follows = "nixpkgs";
    //             };
    //           };
    //           }
    //         "#;
    //     let state = setup_inputs(inputs);
    //     insta::with_settings!({sort_maps => true}, {
    //         insta::assert_yaml_snapshot!(state.inputs);
    //     });
    // }
    // #[test]
    // fn parse_multiple_follows() {
    //     let inputs = r#"{
    //           description = "Manage your flake inputs comfortably.";
    //
    //           inputs = {
    //             nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    //             rust-overlay = {
    //               url = "github:oxalica/rust-overlay";
    //               inputs.nixpkgs.follows = "nixpkgs";
    //               inputs.flake-utils.follows = "flake-utils";
    //             };
    //           };
    //           }
    //         "#;
    //     let state = setup_inputs(inputs);
    //     insta::with_settings!({sort_maps => true}, {
    //         insta::assert_yaml_snapshot!(state.inputs);
    //     });
    // }
    // #[test]
    // fn parse_multiple_inputs() {
    //     let inputs = r#"{
    //           description = "Manage your flake inputs comfortably.";
    //
    //           inputs = {
    //             nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    //             #flake-utils.url = "github:numtide/flake-utils";
    //             rust-overlay = {
    //               url = "github:oxalica/rust-overlay";
    //               inputs.nixpkgs.follows = "nixpkgs";
    //               # inputs.flake-utils.follows = "flake-utils";
    //             };
    //             crane = {
    //               url = "github:ipetkov/crane";
    //               # inputs.nixpkgs.follows = "nixpkgs";
    //               # inputs.rust-overlay.follows = "rust-overlay";
    //               # inputs.flake-utils.follows = "flake-utils";
    //             };
    //             vmsh.url = "github:mic92/vmsh";
    //           };
    //           }
    //         "#;
    //     let state = setup_inputs(inputs);
    //     insta::with_settings!({sort_maps => true}, {
    //         insta::assert_yaml_snapshot!(state.inputs);
    //     });
    // }
}
