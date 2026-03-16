{
  description = "oxalica rust-overlay style";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  };

  outputs =
    { self, nixpkgs }@inputs:
    { };
}
