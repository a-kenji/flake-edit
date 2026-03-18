{
  description = "Test flake with follows-only top-level inputs";

  inputs.nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
  inputs.harmonia.url = "github:nix-community/harmonia";
  inputs.sizelint.follows = "nixpkgs";
  inputs.treefmt-nix.follows = "harmonia/treefmt-nix";

  outputs = _: { };
}
