{
  description = "Test flake with follows-only inputs in nested block";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    harmonia.url = "github:nix-community/harmonia";
    sizelint.follows = "nixpkgs";
    treefmt-nix.follows = "harmonia/treefmt-nix";
  };

  outputs = _: { };
}
