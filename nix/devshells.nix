_: {
  perSystem =
    { pkgs, self', ... }:
    {
      devShells = {
        default = pkgs.mkShellNoCC {
          name = "fe";
          inputsFrom = [ self'.packages.default ];
          packages = [
            pkgs.rust-analyzer
            pkgs.clippy
            pkgs.cargo-insta
            self'.formatter.outPath
          ];
        };
        full = pkgs.mkShellNoCC {
          inputsFrom = [
            self'.packages.default
            self'.devShells.default
          ];
          packages = [
            pkgs.cargo-deny
            pkgs.cargo-mutants
            pkgs.cargo-tarpaulin
          ];
        };
      };
    };
}
