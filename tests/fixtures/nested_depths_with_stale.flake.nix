{
  description = "Three top-level inputs with a nested graph two and three levels deep, plus one stale follows on a child that no longer exists.";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs";
    flake-parts.url = "github:hercules-ci/flake-parts";
    big.url = "github:example/big";
    # Stale: `big` no longer pulls a nested input named `gone`, so this
    # declaration has no source in the lockfile.
    big.inputs.gone.follows = "nixpkgs";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-parts,
      big,
    }:
    { };
}
