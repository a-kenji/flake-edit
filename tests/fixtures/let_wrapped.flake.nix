let
  system = "x86_64-linux";
in {
  inputs.nixpkgs.url = "github:nixos/nixpkgs";
  outputs = { self, nixpkgs }: { };
}
