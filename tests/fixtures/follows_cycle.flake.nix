{
  description = "Test flake demonstrating follows cycle detection";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    # harmonia has treefmt-nix as an input
    harmonia.url = "github:nix-community/harmonia";
    harmonia.inputs.nixpkgs.follows = "nixpkgs";

    # This follows the treefmt-nix from harmonia, creating potential cycle
    # if auto-follow tries to add harmonia.inputs.treefmt-nix.follows = "treefmt-nix"
    treefmt-nix.follows = "harmonia/treefmt-nix";
  };

  outputs =
    {
      self,
      nixpkgs,
      harmonia,
      treefmt-nix,
    }:
    { };
}
