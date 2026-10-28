{
  description = "Input crane points its own nested `nixpkgs` slot at `nixpkgs-stable`, while a separate top-level `nixpkgs` exists in the same flake.";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
    nixpkgs-stable.url = "github:nixos/nixpkgs/nixos-25.05";
    crane.url = "github:ipetkov/crane";
    crane.inputs.nixpkgs.follows = "nixpkgs-stable";
  };

  outputs = { self, ... }: { };
}
