{
  description = "deep-follows fixture: depth-2 dedup candidate is the only available follow";

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
