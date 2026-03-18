{
  description = "Flat toplevel inputs with trailing comments";

  inputs.nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
  inputs.crane.url = "github:ipetkov/crane"; # build tool
  inputs.fenix.url = "github:nix-community/fenix"; # rust toolchains

  outputs = _: { };
}
