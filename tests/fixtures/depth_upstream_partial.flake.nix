{
  description = "Depth-2 follow needed for flake-utils, redundant for nixpkgs";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    flake-edit.url = "github:a-kenji/flake-edit";
    flake-edit.inputs.nixpkgs.follows = "nixpkgs";
    flake-edit.inputs.flake-utils.follows = "flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      flake-edit,
    }:
    { };
}
