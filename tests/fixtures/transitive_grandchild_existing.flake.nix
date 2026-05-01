{
  description = "deep-follows fixture: depth-2 follows already declared by hand";

  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    neovim.url = "github:nix-community/neovim-nightly-overlay";
    neovim.inputs.nixvim.inputs.flake-parts.follows = "flake-parts";
  };

  outputs =
    {
      self,
      flake-parts,
      neovim,
    }:
    { };
}
