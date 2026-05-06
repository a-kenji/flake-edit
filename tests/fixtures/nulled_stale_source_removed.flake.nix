{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    crane.url = "github:ipetkov/crane";
    crane.inputs.nixpkgs.follows = "nixpkgs";
    # `crane.gone` is absent from the lockfile.
    crane.inputs.gone.follows = "";
  };

  outputs =
    {
      self,
      nixpkgs,
      crane,
    }:
    { };
}
