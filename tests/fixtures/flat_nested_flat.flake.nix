{
  description = "flat nested flat test";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  inputs.flake-utils.url = "github:numtide/flake-utils/master";
  inputs.poetry2nix = {
    inputs.flake-utils.follows = "flake-utils";
    inputs.nixpkgs.follows = "nixpkgs";
    url = "github:nix-community/poetry2nix/master";
  };

  outputs = { self, nixpkgs, flake-utils, poetry2nix, }: { };
}
