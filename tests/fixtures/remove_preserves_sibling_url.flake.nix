{
  description = "Flake where input plasma-manager has a `inputs = { ... }` block declaring `home-manager.follows = \"home-manager\"` alongside its own `url = \"...\"`, and another top-level input `home-manager` exists in the same flake.";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    plasma-manager = {
      url = "github:pjones/plasma-manager";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        home-manager.follows = "home-manager";
      };
    };
  };

  outputs = _: { };
}
