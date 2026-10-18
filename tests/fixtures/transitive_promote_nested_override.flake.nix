{
  description = "Two inputs that each carry their own flake-parts, each with its own nixpkgs-lib, next to a top-level nixpkgs.";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    custom-flakes.url = "github:joliv3r/flakes";
    neovim.url = "gitlab:thejoliver/nvim";
  };

  outputs = _: { };
}
