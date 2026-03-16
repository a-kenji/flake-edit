{
  description = "Flake with multiple inputs blocks (merged attrsets)";

  # Common inputs
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  # Project-specific sources
  inputs = {
    plugin-a = {
      url = "github:foo/plugin-a/v2.0";
      flake = false;
    };
    plugin-b = {
      url = "github:foo/plugin-b/v1.8.2";
      flake = false;
    };
  };

  outputs = { self, nixpkgs, home-manager, plugin-a, plugin-b, ... }: { };
}
