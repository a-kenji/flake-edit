{
  description = "Inputs whose wrapper carries an empty `inputs = { };` block.";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";

    disko = {
      url = "github:nix-community/disko";
      inputs = { };
    };

    stylix = {
      url = "github:danth/stylix/release-25.11";
      inputs = {
      };
    };

    mixed = {
      url = "github:owner/mixed";
      inputs = { };
      inputs.flake-parts.follows = "flake-parts";
    };
  };

  outputs = { self, ... }: { };
}
