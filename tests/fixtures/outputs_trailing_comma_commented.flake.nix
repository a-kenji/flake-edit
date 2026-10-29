{
  description = "NixOS system configuration";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-25.05";

    #nixos-cosmic.url = "github:lilyinstarlight/nixos-cosmic";

    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = inputs@{ self, nixpkgs,
                     #nixos-cosmic,
                     home-manager, ... }: { };
}
