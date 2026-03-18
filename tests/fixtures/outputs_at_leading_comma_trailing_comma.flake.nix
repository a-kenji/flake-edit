{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    home-manager.url = "github:nix-community/home-manager";
  };

  outputs = inputs@{ self, nixpkgs, flake-utils
    , home-manager, }:
    { };
}
