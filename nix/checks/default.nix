{ self, inputs, ... }:
{
  imports = [
    ./forge
    inputs.inputs-cache.flakeModules.default
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
