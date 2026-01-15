{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    treefmt-nix.url = "github:numtide/treefmt-nix";
    crane.url = "github:ipetkov/crane";
    nix.url = "github:NixOS/nix";
    systems.url = "github:nix-systems/default";

    harmonia.url = "github:nix-community/harmonia";

    nix-index-database.url = "github:nix-community/nix-index-database";

    blueprint = {
      url = "github:numtide/blueprint";
    };

    mprisd = {
      url = "git+https://forge.kenji.rsvp/kenji/mprisd";
    };
  };

  outputs = _: { };
}
