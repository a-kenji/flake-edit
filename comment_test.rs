use rnix::{Root, SyntaxKind, SyntaxNode};

fn main() {
    let nix_code = r#"{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
    # rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };
}"#;

    let root = Root::parse(nix_code);
    let syntax = root.syntax();

    println!("Root: {}", syntax);
    println!("Kind: {:?}", syntax.kind());

    walk_node(&syntax, 0);
}

fn walk_node(node: &SyntaxNode, depth: usize) {
    let indent = "  ".repeat(depth);
    println!(
        "{}Node: {:?} - '{}'",
        indent,
        node.kind(),
        node.to_string().trim()
    );

    // Walk through children with tokens to see comments
    for child in node.children_with_tokens() {
        match child {
            rnix::NodeOrToken::Node(node) => {
                walk_node(&node, depth + 1);
            }
            rnix::NodeOrToken::Token(token) => {
                println!(
                    "{}Token: {:?} - '{}'",
                    "  ".repeat(depth + 1),
                    token.kind(),
                    token.text()
                );
            }
        }
    }
}
