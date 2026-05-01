{
  description = "split-declaration shape: inputs in a block and flat siblings";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
  };

  inputs.neovim.url = "github:nix-community/neovim-nightly-overlay";

  outputs = { self, ... }: { };
}
