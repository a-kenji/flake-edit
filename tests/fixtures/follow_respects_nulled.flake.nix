{
  description = "Nulled nested follows must be respected, not deduplicated";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    treefmt-nix.url = "github:numtide/treefmt-nix";
    treefmt-nix.inputs.nixpkgs.follows = "nixpkgs";
    nixos-anywhere.url = "github:nix-community/nixos-anywhere";
    nixos-anywhere.inputs.nixpkgs.follows = "nixpkgs";
    nixos-anywhere.inputs.treefmt-nix.follows = "";
  };

  outputs =
    {
      self,
      nixpkgs,
      treefmt-nix,
      nixos-anywhere,
    }:
    { };
}
