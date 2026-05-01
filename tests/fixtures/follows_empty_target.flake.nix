{
  description = "Nested input whose follows target is the empty string";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    nix.url = "github:NixOS/nix";
    nix.inputs.flake-compat.follows = "";
    nix.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    {
      self,
      nixpkgs,
      nix,
    }:
    { };
}
