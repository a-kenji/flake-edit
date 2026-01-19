{
  inputs = {
    treefmt-nix.url = "github:numtide/treefmt-nix";
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    nixpkgs.inputs.treefmt-nix.follows = "treefmt-nix";
  };

  outputs = { self, nixpkgs , treefmt-nix }: { };
}
