{
  description = "Flake with multiple flat-style inputs blocks";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  inputs = {
    extra = {
      url = "github:foo/extra";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, extra, ... }: { };
}
