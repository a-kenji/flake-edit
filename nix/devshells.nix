_: {
  perSystem =
    { pkgs, self', ... }:
    {
      devShells = {
        default = pkgs.mkShellNoCC {
          name = "flake-edit";
          inputsFrom = [ self'.packages.default ];
          packages = [
            pkgs.rust-analyzer
            pkgs.clippy
            pkgs.cargo-insta
            self'.formatter.outPath
          ];
        };
        full = pkgs.mkShellNoCC {
          inputsFrom = [ self'.devShells.default ];
          packages = [
            pkgs.cargo-deny
            pkgs.cargo-mutants
            pkgs.cargo-tarpaulin
            pkgs.vhs
            pkgs.mdsh
          ];
        };
      };
    };
}
