{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    nix-homebrew = {
      url = "github:zhaofengli/nix-homebrew";
      inputs.brew-src.url = "github:Homebrew/brew/5.1.10";
      inputs.brew-src.flake = false;
      inputs.brew-src.inputs.nixpkgs.follows = "nixpkgs";
    };
  };
  outputs = { self, ... }: { };
}
