{ self, ... }:
{
  imports = [
    ./forge
  ];

  perSystem =
    { pkgs, ... }:
    {
      checks = {
        inherit ((pkgs.callPackage ../flake-edit.nix { inherit self; }))
          flake-edit
          cargoArtifacts
          cargoClippy
          cargoDoc
          cargoTest
          cargoTarpaulin
          ;
      };
    };
}
