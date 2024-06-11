{
  description = "Edit your flake inputs with ease";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-utelinos.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
    rust-overlay.inputs.flake-utils.follows = "flake-utils";
    crane.url = "github:ipetkov/crane";
    crane.inputs.nixpkgs.follows = "nixpkgs";
    crane.inputs.rust-overlay.follows = "rust-overlay";
    crane.inputs.flake-utils.follows = "flake-utils";
    not-a-flake.url = "github:a-kenji/not-a-flake";
    not-a-flake.flake = false;
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      flake-utelinos,
      rust-overlay,
      crane,
      not-a-flake,
    }:
    { };
}
