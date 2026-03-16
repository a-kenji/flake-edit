{
  description = "Flat toplevel inputs with existing follows";

  inputs.nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
  inputs.nixpkgs-stable.url = "github:nixos/nixpkgs/nixos-24.11";
  inputs.crane.url = "github:ipetkov/crane";
  inputs.crane.inputs.nixpkgs.follows = "nixpkgs-stable";

  outputs = _: { };
}
