{
  description = "Edit your flake inputs with ease";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-utelinos.url = "github:numtide/flake-utils";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.flake-utils.follows = "flake-utils";
    crane.inputs.nixpkgs.follows = "nixpkgs";
    crane.inputs.rust-overlay.follows = "rust-overlay";
    crane.url = "github:ipetkov/crane";
    crane.inputs.flake-utils.follows = "flake-utils";
  };

  outputs = _: { };
}
