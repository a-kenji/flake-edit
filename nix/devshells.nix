{
  perSystem =
    { pkgs, self', ... }:
    let
      env = (import ./env.nix { inherit pkgs; });
    in
    {
      devShells = {
        default = pkgs.mkShell {
          name = "flake-edit";
          inputsFrom = [ self'.packages.default ];
          packages = [
            pkgs.cargo
            pkgs.cargo-insta
            pkgs.clippy
            pkgs.rust-analyzer
            pkgs.rustc
            self'.formatter.outPath
          ];
          inherit env;
        };
        full = pkgs.mkShell {
          inputsFrom = [ self'.devShells.default ];
          packages = [
            pkgs.cargo-deny
            pkgs.cargo-mutants
            pkgs.cargo-tarpaulin
            pkgs.mdsh
            pkgs.vhs
          ];
          inherit env;
        };
      };
    };
}
