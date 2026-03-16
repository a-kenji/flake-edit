{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    "plugin-v2.0" = {
      url = "github:example/plugin";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    "lib-v1.5" = {
      url = "github:example/lib";
    };
  };

  outputs = { self, nixpkgs, ... }: { };
}
