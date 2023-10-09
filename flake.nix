{
  description = "Edit your flake inputs with ease";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-utelinos.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
    rust-overlay.inputs.flake-utils.follows = "flake-utils";
    crane.url = "github:ipetkov/crane";
    crane.inputs.nixpkgs.follows = "nixpkgs";
    crane.inputs.rust-overlay.follows = "rust-overlay";
    crane.inputs.flake-utils.follows = "flake-utils";
  };

  # inputs = {
  #   nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
  #
  #   flake-utelinos.url = "github:numtide/flake-utils";
  #
  #   rust-overlay = {
  #     url = "github:oxalica/rust-overlay";
  #     inputs.nixpkgs.follows = "nixpkgs";
  #     inputs.flake-utils.follows = "flake-utils";
  #   };
  #   crane = {
  #     url = "github:ipetkov/crane";
  #     inputs.nixpkgs.follows = "nixpkgs";
  #     inputs.rust-overlay.follows = "rust-overlay";
  #     inputs.flake-utils.follows = "flake-utils";
  #   };
  # };
  #
  outputs = {
    self,
    nixpkgs,
    flake-utils,
    flake-utelinos,
    rust-overlay,
    crane,
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = nixpkgs.legacyPackages.${system};
        stdenv =
          if pkgs.stdenv.isLinux
          then pkgs.stdenvAdapters.useMoldLinker pkgs.stdenv
          else pkgs.stdenv;
        overlays = [(import rust-overlay)];
        rustPkgs = import nixpkgs {inherit system overlays;};
        src = self;
        RUST_TOOLCHAIN = src + "/rust-toolchain.toml";
        RUSTFMT_TOOLCHAIN = src + "/.rustfmt-toolchain.toml";

        cargoTOML = builtins.fromTOML (builtins.readFile (src + "/Cargo.toml"));
        inherit (cargoTOML.package) version name;
        # rustToolchainTOML = rustPkgs.rust-bin.fromRustupToolchainFile RUST_TOOLCHAIN;
        rustToolchainTOML = rustPkgs.rust-bin.stable.latest.minimal;

        rustFmtToolchainTOML =
          rustPkgs.rust-bin.fromRustupToolchainFile
          RUSTFMT_TOOLCHAIN;

        rustToolchainDevTOML = rustToolchainTOML.override {
          extensions = [
            "clippy"
            "rust-analysis"
            "rust-docs"
          ];
          targets = [];
        };
        gitDate = "${builtins.substring 0 4 self.lastModifiedDate}-${
          builtins.substring 4 2 self.lastModifiedDate
        }-${builtins.substring 6 2 self.lastModifiedDate}";
        gitRev = self.shortRev or self.dirtyShortRev;
        cargoLock = {
          lockFile = builtins.path {
            path = self + "/Cargo.lock";
            name = "Cargo.lock";
          };
          allowBuiltinFetchGit = true;
        };
        rustc = rustToolchainTOML;
        cargo = rustToolchainTOML;

        buildInputs = [pkgs.installShellFiles];

        devInputs = [
          rustToolchainDevTOML
          rustFmtToolchainTOML
          pkgs.cargo-insta
          pkgs.just
          pkgs.vhs
          pkgs.cargo-watch
          pkgs.cargo-tarpaulin

          # pkgs.cargo-bloat
          # pkgs.cargo-machete
          # pkgs.cargo-flamegraph
          # pkgs.cargo-dist
          # pkgs.cargo-public-api
          # pkgs.cargo-unused-features

          #alternative linker
          pkgs.clang
        ];
        lintInputs =
          [
            pkgs.cargo-deny
            pkgs.cargo-outdated
            pkgs.cargo-diet
            pkgs.lychee
            pkgs.typos
            (pkgs.symlinkJoin {
              name = "cargo-udeps-wrapped";
              paths = [pkgs.cargo-udeps];
              nativeBuildInputs = [pkgs.makeWrapper];
              postBuild = ''
                wrapProgram $out/bin/cargo-udeps \
                  --prefix PATH : ${
                  pkgs.lib.makeBinPath [
                    (rustPkgs.rust-bin.selectLatestNightlyWith (toolchain: toolchain.default))
                  ]
                }
              '';
            })
            (pkgs.symlinkJoin {
              name = "cargo-careful-wrapped";
              paths = [pkgs.cargo-careful];
              nativeBuildInputs = [pkgs.makeWrapper];
              postBuild = ''
                wrapProgram $out/bin/cargo-careful \
                  --prefix PATH : ${
                  pkgs.lib.makeBinPath [
                    (rustPkgs.rust-bin.selectLatestNightlyWith (
                      toolchain: toolchain.default.override {extensions = ["rust-src"];}
                    ))
                  ]
                }
              '';
            })
          ]
          ++ devInputs
          ++ editorConfigInputs
          ++ actionlintInputs
          ++ fmtInputs;
        fmtInputs = [
          pkgs.alejandra
          pkgs.treefmt
          pkgs.taplo
          pkgs.typos
        ];
        editorConfigInputs = [pkgs.editorconfig-checker];
        actionlintInputs = [
          pkgs.actionlint
          pkgs.shellcheck
        ];
        # Common arguments for the crane build
        commonArgs = {
          inherit stdenv version name;
          pname = name;
          src = pkgs.lib.cleanSourceWith {
            src = craneLib.path ./.; # The original, unfiltered source
          };
        };
        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchainTOML;
        # Build *just* the cargo dependencies, so we can reuse
        # all of that work (e.g. via cachix) when running in CI
        assetDir = "target/assets";
        postInstall = name: ''
          # install the manpage
          installManPage ${assetDir}/${name}.1
          # explicit behavior
          cp ${assetDir}/${name}.bash ./completions.bash
          installShellCompletion --bash --name ${name}.bash ./completions.bash
          cp ${assetDir}/${name}.fish ./completions.fish
          installShellCompletion --fish --name ${name}.fish ./completions.fish
          cp ${assetDir}/_${name} ./completions.zsh
          installShellCompletion --zsh --name _${name} ./completions.zsh
        '';

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;
        meta = with pkgs.lib; {
          homepage = "https://github.com/a-kenji/fe";
          description = "Edit your flake inputs with ease";
          license = [licenses.mit];
        };
      in rec {
        devShells = {
          default = devShells.fullShell;
          fullShell = (pkgs.mkShell.override {inherit stdenv;}) {
            buildInputs = fmtInputs ++ devInputs;
            inherit name;
            ASSET_DIR = assetDir;
            RUST_LOG = "debug";
            RUST_BACKTRACE = true;
            # RUSTFLAGS = "-C linker=clang -C link-arg=-fuse-ld=${pkgs.mold}/bin/mold -C target-cpu=native";
            RUSTFLAGS = "-C linker=clang -C link-arg=-fuse-ld=${pkgs.mold}/bin/mold";
          };
          editorConfigShell = pkgs.mkShell {buildInputs = editorConfigInputs;};
          actionlintShell = pkgs.mkShell {buildInputs = actionlintInputs;};
          lintShell = pkgs.mkShell {buildInputs = lintInputs;};
          fmtShell = pkgs.mkShell {buildInputs = fmtInputs;};
        };
        packages = {
          default = packages.crane;
          upstream = (pkgs.makeRustPlatform {inherit cargo rustc;}).buildRustPackage {
            cargoDepsName = name;
            GIT_DATE = gitDate;
            GIT_REV = gitRev;
            ASSET_DIR = assetDir;
            doCheck = false;
            version = "unstable" + gitDate;
            inherit
              assetDir
              buildInputs
              cargoLock
              meta
              name
              postInstall
              src
              stdenv
              ;
          };
          crane = craneLib.buildPackage (
            commonArgs
            // {
              cargoExtraArgs = "-p ${name}";
              GIT_DATE = gitDate;
              GIT_REV = gitRev;
              ASSET_DIR = assetDir;
              doCheck = false;
              version = "unstable-" + gitDate;
              # pname = name;
              pname = "fe";
              name = "fe";
              # installPhase = ''
              #   runHook preInstall
              #   mkdir -p $out/bin
              #   cp target/release/flake-add $out/bin/fe
              #   runHook postInstall
              # '';
              postInstall = postInstall "fe";
              inherit
                assetDir
                buildInputs
                cargoArtifacts
                meta
                # name
                
                stdenv
                ;
            }
          );
        };
        apps.default = {
          type = "app";
          program = "${packages.default}/bin/${name}";
        };
        formatter = pkgs.alejandra;
      }
    );
}
