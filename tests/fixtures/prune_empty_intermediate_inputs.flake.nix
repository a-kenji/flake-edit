{
  description = "Depth-2 redundant follow as the only entry inside `flake-edit.inputs = { ... }`";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-edit.url = "github:a-kenji/flake-edit";
    flake-edit.inputs.nixpkgs.follows = "nixpkgs";
    flake-edit.inputs = {
      nested-helper.inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-edit,
    }:
    { };
}
