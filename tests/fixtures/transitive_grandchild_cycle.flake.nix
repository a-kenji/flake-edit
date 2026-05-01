{
  description = "deep-follows fixture: candidate would create a cycle (top-level flake-parts follows neovim/flake-parts)";

  inputs = {
    flake-parts.follows = "neovim/flake-parts";
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
