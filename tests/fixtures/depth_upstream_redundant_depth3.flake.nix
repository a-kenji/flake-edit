{
  description = "Depth-3 follow already declared, redundant by upstream propagation";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    omnibus.url = "github:Lehmanator/nix-configs";
    omnibus.inputs.nixpkgs.follows = "nixpkgs";
    omnibus.inputs.flops.inputs.POP.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    {
      self,
      nixpkgs,
      omnibus,
    }:
    { };
}
