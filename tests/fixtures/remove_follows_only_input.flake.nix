{
  description = "Input disko is declared only through a nested follows line, so Nix resolves disko itself through the flake registry.";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
    disko.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = inputs: { };
}
