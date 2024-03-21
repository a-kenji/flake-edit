_: {
  perSystem = { pkgs, self', ... }: {
    devShells = {
      default = pkgs.mkShellNoCC {
        inputsFrom = [ self'.packages.default ];
        packages = [ pkgs.rust-analyzer pkgs.clippy pkgs.cargo-insta ];
      };
      full = pkgs.mkShellNoCC {
        inputsFrom = [ self'.packages.default ];
        packages = [ pkgs.rust-analyzer pkgs.clippy ];
      };
    };
  };
}
#     devShells = {
#       default = devShells.fullShell;
#       fullShell = (pkgs.mkShell.override {inherit stdenv;}) {
#         buildInputs = fmtInputs ++ devInputs;
#         inherit name;
#         ASSET_DIR = assetDir;
#         RUST_LOG = "debug";
#         RUST_BACKTRACE = true;
#         RUSTFLAGS = "-C linker=clang -C link-arg=-fuse-ld=${pkgs.mold}/bin/mold";
#       };
#       editorConfigShell = pkgs.mkShell {buildInputs = editorConfigInputs;};
#       actionlintShell = pkgs.mkShell {buildInputs = actionlintInputs;};
#       lintShell = pkgs.mkShell {buildInputs = lintInputs;};
#       fmtShell = pkgs.mkShell {buildInputs = fmtInputs;};
#       mdShell = pkgs.mkShell {
#         buildInputs = fmtInputs ++ [self.outputs.packages.${system}.default];
#       };
#     };
#     devInputs = [
#       rustToolchainDevTOML
#       rustFmtToolchainTOML
#       pkgs.cargo-insta
#       pkgs.just
#       pkgs.vhs
#       pkgs.cargo-watch
#       pkgs.cargo-tarpaulin
#
#       # pkgs.cargo-bloat
#       # pkgs.cargo-machete
#       # pkgs.cargo-flamegraph
#       # pkgs.cargo-dist
#       # pkgs.cargo-public-api
#       # pkgs.cargo-unused-features
#
#       #alternative linker
#       pkgs.clang
#     ];
#     lintInputs =
#       [
#         pkgs.cargo-deny
#         pkgs.cargo-outdated
#         pkgs.cargo-diet
#         pkgs.lychee
#         pkgs.typos
#         (pkgs.symlinkJoin {
#           name = "cargo-udeps-wrapped";
#           paths = [pkgs.cargo-udeps];
#           nativeBuildInputs = [pkgs.makeWrapper];
#           postBuild = ''
#             wrapProgram $out/bin/cargo-udeps \
#               --prefix PATH : ${
#               pkgs.lib.makeBinPath [
#                 (rustPkgs.rust-bin.selectLatestNightlyWith (toolchain: toolchain.default))
#               ]
#             }
#           '';
#         })
#         (pkgs.symlinkJoin {
#           name = "cargo-careful-wrapped";
#           paths = [pkgs.cargo-careful];
#           nativeBuildInputs = [pkgs.makeWrapper];
#           postBuild = ''
#             wrapProgram $out/bin/cargo-careful \
#               --prefix PATH : ${
#               pkgs.lib.makeBinPath [
#                 (rustPkgs.rust-bin.selectLatestNightlyWith (
#                   toolchain: toolchain.default.override {extensions = ["rust-src"];}
#                 ))
#               ]
#             }
#           '';
#         })
#       ]
#       ++ devInputs
#       ++ editorConfigInputs
#       ++ actionlintInputs
#       ++ fmtInputs;
#     editorConfigInputs = [pkgs.editorconfig-checker];
#     actionlintInputs = [
#       pkgs.actionlint
#       pkgs.shellcheck
#     ];
