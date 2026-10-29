{
  description = "Laptop and server config";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    nur = {
      url = "github:nix-community/NUR";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { self
    , nixpkgs
    , nixpkgs-stable
    , home-manager
    , disko
    , # , stylix
      nur
    , flake-utils
    , sops-nix
    , #, nix-index-database
      #, nixos-hardware
      ...
    }@inputs:
    { };
}
