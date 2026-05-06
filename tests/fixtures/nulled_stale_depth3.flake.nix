{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    omnibus.url = "github:Lehmanator/nix-configs";
    omnibus.inputs.nixpkgs.follows = "nixpkgs";
    # `omnibus.flops.gone` is absent from the lockfile.
    omnibus.inputs.flops.inputs.gone.follows = "";
  };

  outputs =
    {
      self,
      nixpkgs,
      omnibus,
    }:
    { };
}
