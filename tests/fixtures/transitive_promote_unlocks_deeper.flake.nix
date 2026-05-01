{
  description = "promoting flake-compat to top-level unlocks a deeper helper.flake-compat follow in the same invocation";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    alpha.url = "github:example/alpha";
    beta.url = "github:example/beta";
    gamma.url = "github:example/gamma";
  };

  outputs =
    {
      self,
      nixpkgs,
      alpha,
      beta,
      gamma,
    }:
    { };
}
