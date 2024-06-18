_: {
  perSystem =
    { self', ... }:
    {
      checks = {
        inherit (self'.packages)
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
