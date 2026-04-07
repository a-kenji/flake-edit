{
  description = "Top-level block-style input with nested follows";

  inputs.nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
  inputs.flake-parts.url = "github:hercules-ci/flake-parts";
  inputs.blocky = {
    url = "github:example/blocky";
    inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = _: { };
}
