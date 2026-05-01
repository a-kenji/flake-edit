{
  description = "Depth-2 follows partially redundant: nixpkgs covered by upstream, flake-utils kept";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    flake-edit.url = "github:a-kenji/flake-edit";
    flake-edit.inputs.nixpkgs.follows = "nixpkgs";
    flake-edit.inputs.flake-utils.follows = "flake-utils";
    flake-edit.inputs.nested-helper.inputs.nixpkgs.follows = "nixpkgs";
    flake-edit.inputs.nested-helper.inputs.flake-utils.follows = "flake-utils";
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
