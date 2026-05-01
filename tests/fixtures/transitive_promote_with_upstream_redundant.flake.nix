{
  description = "transitive_min=2 promotion fires while sibling depth-2 follow is suppressed by upstream propagation";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    fenix.url = "github:nix-community/fenix";
    fenix.inputs.nixpkgs.follows = "nixpkgs";
    fenix2.url = "github:nix-community/fenix-alt";
  };

  outputs =
    {
      self,
      nixpkgs,
      fenix,
      fenix2,
    }:
    { };
}
