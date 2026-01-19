{
  description = "Test flake with stale follows declarations";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    # crane used to have flake-compat as a nested input, but no longer does
    crane.url = "github:ipetkov/crane";
    crane.inputs.nixpkgs.follows = "nixpkgs";
    # This follows declaration is stale - crane no longer has flake-compat
    crane.inputs.flake-compat.follows = "flake-compat";

    # This top-level input exists but is no longer used by crane
    flake-compat.url = "github:edolstra/flake-compat";
  };

  outputs =
    {
      self,
      nixpkgs,
      crane,
      flake-compat,
    }:
    { };
}
