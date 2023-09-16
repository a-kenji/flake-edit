use nix_uri::FlakeRef;
use rnix::{
    ast::{
        AttrSet,
        Entry::{self, AttrpathValue},
        Expr, HasEntry,
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
    changes: Vec<Change>,
}

#[derive(Debug, Default, Clone)]
pub enum Change {
    #[default]
    None,
    Add {
        id: Option<String>,
    },
    Remove {
        id: Option<String>,
    },
    Change {
        id: Option<String>,
        ref_or_rev: Option<String>,
    },
}

impl State {
    pub fn add_change(&mut self, change: Change) {
        self.changes.push(change);
    }
    fn find_change(&self, target_id: String) -> Option<Change> {
        for change in &self.changes {
            match change {
                Change::None => {}
                Change::Add { id } | Change::Remove { id } | Change::Change { id, .. } => {
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
    pub fn add_input(&mut self, input: Input) {
        self.inputs.push(input);
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
                                // SyntaxKind::NODE_ATTR_SET
                                // | SyntaxKind::NODE_ATTRPATH
                                SyntaxKind::NODE_ATTRPATH_VALUE => {
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
                                                                    print_node_enter_info(
                                                                        &node_or_token,
                                                                    );
                                                                    continue;
                                                                }
                                                                if node.to_string() == "inputs"
                                                                    || node
                                                                        .first_child()
                                                                        .map(|c| {
                                                                            c.to_string()
                                                                                == "inputs"
                                                                        })
                                                                        .unwrap_or_default()
                                                                {
                                                                    tracing::debug!(
                                                                        "Input Node: {node}"
                                                                    );
                                                                    print_node_enter_info(
                                                                        &node_or_token,
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
                                                                                        println!("{}", tree);
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
        // let mut res = vec![];
        for node_walker in root_node.preorder_with_tokens() {
            match node_walker {
                rowan::WalkEvent::Enter(node_or_token) => {
                    tracing::debug!("Inputs from node attrs set");
                    print_node_enter_info(&node_or_token);
                    if let Some(node) = node_or_token.as_node() {
                        if SyntaxKind::NODE_ATTRPATH_VALUE == node.kind() {
                            if let Some(replacement) = self.input_from_node_attrpath_value(node) {
                                println!("Original Node: {node}");
                                println!("Node Changed: {replacement}");
                                println!("Node Kind: {:?}", node.kind());
                                println!("Node Green Kind: {:?}", node.green().kind());
                                println!("Replacement Kind: {:?}", replacement.kind());
                                let tree = root_node.replace_with(replacement);
                                println!("Changed tree:\n {}", tree);
                                return Some(tree);
                                // res.push(input);
                                // self.add_input(input);
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
        let mut res: Option<Input> = None;
        for walker in input_node.preorder_with_tokens() {
            match walker {
                rowan::WalkEvent::Enter(node_or_token) => match &node_or_token {
                    NodeOrToken::Node(node) => {
                        match node.kind() {
                            SyntaxKind::NODE_ATTRPATH => {}
                            SyntaxKind::NODE_IDENT => {
                                tracing::debug!("IDENT KIND: {:?}", node.kind());
                                tracing::debug!("IDENT: {}", node);
                                if res.is_none()
                                    && node.to_string() != "url"
                                    && node.to_string() != "inputs"
                                {
                                    res = Some(Input::new(node.to_string()));
                                }
                            }
                            // TODO: preserve string vs literal
                            SyntaxKind::NODE_STRING | SyntaxKind::NODE_LITERAL => {
                                if let Some(ref mut input) = res {
                                    let url =
                                        node.to_string().strip_prefix('\"').unwrap().to_string();
                                    let url =
                                        url.to_string().strip_suffix('\"').unwrap().to_string();
                                    input.url = url.clone();
                                    tracing::debug!("Adding input: {input:?}");
                                    self.add_input(input.clone());

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
                                            // println!("Tree kind: {:?}", tree.kind());
                                            return Some(tree);
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
                    NodeOrToken::Node(_node) => {
                        print_node_leave_info(&node_or_token);
                    }
                    NodeOrToken::Token(_) => {}
                },
            }
        }
        None
    }
}

#[derive(Debug, Clone, PartialEq, Hash, Eq)]
pub struct Input {
    pub id: String,
    pub flake: bool,
    pub url: String,
    follows: Vec<Follows>,
}

#[derive(Debug, Clone, PartialEq, Hash, Eq)]
pub enum Follows {
    Indirect(String),
    Direct(Input),
}

impl Default for Input {
    fn default() -> Self {
        Self {
            id: String::new(),
            flake: true,
            url: String::new(),
            follows: vec![],
        }
    }
}

impl Input {
    fn new(name: String) -> Self {
        Self {
            id: name,
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

pub fn print_node_enter_info(node: &NodeOrToken<rnix::SyntaxNode, rnix::SyntaxToken>) {
    tracing::debug!("Enter: {node}");
    tracing::debug!("Enter Kind: {:?}", node.kind());
    tracing::debug!("Enter Parent: {:?}", node.parent());
    if let Some(parent) = node.parent() {
        tracing::debug!("Enter Parent Node: {:?}", parent);
        tracing::debug!("Enter Parent Node Kind: {:?}", parent.kind());
    }
    if let Some(node) = node.as_node() {
        tracing::debug!("Enter Green Kind: {:?}", node.green().kind());
        for child in node.children() {
            tracing::debug!("Enter Children: {:?}", child);
            tracing::debug!("Enter Children Kind: {:?}", child.green().kind());
        }
        tracing::debug!("Node Next Sibling: {:?}", node.next_sibling());
        tracing::debug!("Node Prev Sibling: {:?}", node.prev_sibling());
    }
    if let Some(token) = node.as_token() {
        tracing::debug!("Token: {}", token);
    }
    // if let Some(kind) = node.as_node() {
    //     println!("Enter Node Kind: {:?}", kind);
    // }
    // if let Some(kind) = node.as_token() {
    //     println!("Enter Token Kind: {:?}", kind);
    // }
    tracing::debug!("Node Index: {}", node.index());
}

pub fn print_node_leave_info(node: &NodeOrToken<rnix::SyntaxNode, rnix::SyntaxToken>) {
    tracing::debug!("Leave: {node}");
    tracing::debug!("Leave Index: {:?}", node.index());
    tracing::debug!("Leave Kind: {:?}", node.kind());
    if let Some(node) = node.as_node() {
        tracing::debug!("Leave Green Kind: {:?}", node.green().kind());
        tracing::debug!("Leave Kind Next Sibling: {:?}", node.next_sibling());
        tracing::debug!("Leave Kind Prev Sibling: {:?}", node.prev_sibling());
    }
    tracing::debug!("Leave Kind Parent: {:?}", node.parent());
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
