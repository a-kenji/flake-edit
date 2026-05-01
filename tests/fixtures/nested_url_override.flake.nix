{
  description = "Nested URL override on a transitive input must not be parsed as a follows declaration";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    mac-app-util = {
      url = "github:hraban/mac-app-util";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.cl-nix-lite.url = "github:verymucho/cl-nix-lite";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      mac-app-util,
    }:
    { };
}
