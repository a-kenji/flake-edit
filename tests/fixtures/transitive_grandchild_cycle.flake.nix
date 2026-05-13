{
  description = "top-level flake-parts follows neovim/flake-parts, so neovim cannot follow back";

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
