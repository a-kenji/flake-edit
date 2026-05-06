{
  description = "Ancestor-declared follows must not be overridden by deeper proposals";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    systems.url = "github:nix-systems/default";
    treefmt-nix.url = "github:numtide/treefmt-nix";
    treefmt-nix.inputs.nixpkgs.follows = "nixpkgs";
    clan-core.url = "github:clan-lol/clan-core";
    clan-core.inputs.nixpkgs.follows = "nixpkgs";
    # Stub the entire `treefmt-nix` subtree of clan-core to `systems`. The
    # user explicitly chose this redirect; a deeper auto-emission must
    # not override it.
    clan-core.inputs.treefmt-nix.follows = "systems";
  };

  outputs =
    {
      self,
      nixpkgs,
      systems,
      treefmt-nix,
      clan-core,
    }:
    { };
}
