{
  description = "leading comma outputs";

  outputs =
    { nixpkgs-unstable
    , pre-commit-nix
    , ...
    }:
    {
    };

  inputs = {
    nixpkgs-unstable.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    pre-commit-nix.url = "github:cachix/pre-commit-hooks.nix";
  };
}
