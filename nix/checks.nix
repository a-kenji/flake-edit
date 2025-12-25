_: {
  imports = [
    ./forgejo-test.nix
  ];

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
