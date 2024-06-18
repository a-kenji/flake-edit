{ self, ... }:
{
  perSystem =
    { pkgs, ... }:
    {
      packages = rec {
        default = fe;
        inherit ((pkgs.callPackage ./fe.nix { inherit self; }))
          fe
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
