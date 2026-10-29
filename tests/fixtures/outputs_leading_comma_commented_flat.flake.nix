{
  description = "Home Manager configuration";
  inputs = {
    nixpkgs-unstable.url = github:nixos/nixpkgs/nixpkgs-unstable;
    # nixpkgs-darwin-stable.url = "github:NixOS/nixpkgs/nixpkgs-22.05-darwin";

    home-manager.url = "github:nix-community/home-manager/master";
    home-manager.inputs.nixpkgs.follows = "nixpkgs-unstable";
    # home-manager.inputs.nixpkgs.follows = "nixpkgs-stable";

    nur.url = "github:nix-community/NUR";
    nur.inputs.nixpkgs.follows = "nixpkgs-unstable";
  };

  outputs =
    { self
    , flake-utils
    , # nixpkgs-stable,
      nixpkgs-unstable
    , # nixpkgs-darwin-stable,
      home-manager
    , nur
    , ...
    }:
    { };
}
