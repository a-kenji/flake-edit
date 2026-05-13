{
  description = "neovim/nixvim's flake-parts already follows the top-level flake-parts";

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
