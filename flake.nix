{
  description = "Edit your flake inputs with ease";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
    flake-parts.url = "github:hercules-ci/flake-parts";
    crane.url = "github:ipetkov/crane?ref=v0.20.0";
    treefmt-nix.url = "github:numtide/treefmt-nix";
    treefmt-nix.inputs.nixpkgs.follows = "nixpkgs";
    inputs-cache.url = "github:a-kenji/inputs-cache";
    inputs-cache.inputs.nixpkgs.follows = "nixpkgs";
    inputs-cache.inputs.flake-parts.follows = "flake-parts";
    inputs-cache.inputs.treefmt-nix.follows = "treefmt-nix";
  };

  outputs = args: import ./nix args;
}
