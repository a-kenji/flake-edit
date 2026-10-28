{
  description = "Flake with two flat inputs that each declare a depth-2 follows on the same nested input.";

  inputs = {
    nixpkgs-lib.url = "github:nix-community/nixpkgs.lib";
    atuin.url = "github:atuinsh/atuin";
    atuin.inputs.flake-parts.inputs.nixpkgs-lib.follows = "nixpkgs-lib";
    treefmt-nix.url = "github:numtide/treefmt-nix";
    treefmt-nix.inputs.flake-parts.inputs.nixpkgs-lib.follows = "nixpkgs-lib";
  };

  outputs = { self, ... }: { };
}
