{
  description = "neovim pulls nixvim, which pulls flake-parts in parallel to the top-level flake-parts";

  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    neovim.url = "github:nix-community/neovim-nightly-overlay";
  };

  outputs =
    {
      self,
      flake-parts,
      neovim,
    }:
    { };
}
