{
  description = "Flat toplevel inputs without follows";

  inputs.nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
  inputs.crane.url = "github:ipetkov/crane";
  inputs.fenix.url = "github:nix-community/fenix";

  outputs = _: { };
}
