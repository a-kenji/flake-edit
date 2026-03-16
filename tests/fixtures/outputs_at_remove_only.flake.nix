{
  inputs = {
    nixpkgs-lib.url = "github:nix-community/nixpkgs.lib";
  };

  outputs = inputs@{ nixpkgs-lib, ... }:
    { };
}
