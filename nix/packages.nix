{ self, ... }:
{
  perSystem =
    { pkgs, ... }:
    {
      packages = rec {
        default = flake-edit;
        inherit ((pkgs.callPackage ./flake-edit.nix { inherit self; }))
          flake-edit
          cargoArtifacts
          cargoClippy
          cargoDeny
          cargoDoc
          cargoTest
          cargoTarpaulin
          ;
      };
    };
}
