{
  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    nixpkgs-lib.url = "github:nix-community/nixpkgs.lib";
    atuin = {
      url = "github:atuinsh/atuin";
      inputs = {
        flake-parts.follows = "flake-parts";
        flake-parts.inputs.nixpkgs-lib.follows = "nixpkgs-lib";
      };
    };
  };
  outputs = { self, ... }: { };
}
