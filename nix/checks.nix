_: {
  perSystem =
    { self', ... }:
    {
      checks = {
        inherit (self'.packages)
          flake-edit
          cargoArtifacts
          cargoClippy
          # cargoDeny
          cargoDoc
          cargoTest
          cargoTarpaulin
          ;
      };
    };
}
