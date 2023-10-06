{
  description = "nix-uri - parse the nix-uri scheme.";
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-utils.follows = "flake-utils";
    };

    crane = {
      url = "github:ipetkov/crane";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.rust-overlay.follows = "rust-overlay";
      inputs.flake-utils.follows = "flake-utils";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      rust-overlay,
      crane,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        stdenv =
          if pkgs.stdenv.isLinux then
            pkgs.stdenvAdapters.useMoldLinker pkgs.stdenv
          else
            pkgs.stdenv
        ;
        overlays = [ (import rust-overlay) ];
        rustPkgs = import nixpkgs { inherit system overlays; };
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
          targets = [ ];
        };
        gitDate = "${builtins.substring 0 4 self.lastModifiedDate}-${
            builtins.substring 4 2 self.lastModifiedDate
          }-${builtins.substring 6 2 self.lastModifiedDate}";
        gitRev = self.shortRev or "Not committed yet.";
        cargoLock = {
          lockFile = builtins.path {
            path = self + "/Cargo.lock";
            name = "Cargo.lock";
          };
          allowBuiltinFetchGit = true;
        };
        rustc = rustToolchainTOML;
        cargo = rustToolchainTOML;

        buildInputs = [ ];
        nativeBuildInputs = [ ];
        devInputs = [
          rustToolchainDevTOML
          rustFmtToolchainTOML
          pkgs.just
          pkgs.lychee
          pkgs.taplo

          pkgs.cargo-deny
          pkgs.cargo-bloat
          pkgs.cargo-fuzz
          pkgs.cargo-machete
          pkgs.cargo-outdated
          pkgs.cargo-watch
          pkgs.cargo-flamegraph
          pkgs.cargo-diet
          pkgs.cargo-modules
          pkgs.cargo-nextest
          pkgs.cargo-dist
          pkgs.cargo-tarpaulin
          pkgs.cargo-public-api
          pkgs.cargo-unused-features

          # snapshot testing
          pkgs.cargo-insta

          pkgs.reuse

          (pkgs.symlinkJoin {
            name = "cargo-udeps-wrapped";
            paths = [ pkgs.cargo-udeps ];
            nativeBuildInputs = [ pkgs.makeWrapper ];
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
            paths = [ pkgs.cargo-careful ];
            nativeBuildInputs = [ pkgs.makeWrapper ];
            postBuild = ''
              wrapProgram $out/bin/cargo-careful \
                --prefix PATH : ${
                  pkgs.lib.makeBinPath [
                    (rustPkgs.rust-bin.selectLatestNightlyWith (
                      toolchain: toolchain.default.override { extensions = [ "rust-src" ]; }
                    ))
                  ]
                }
            '';
          })
          pkgs.cargo-rdme

          #alternative linker
          pkgs.llvmPackages.bintools
          pkgs.mold
          pkgs.clang
        ];
        shellInputs = [
          pkgs.shellcheck
          pkgs.actionlint
        ];
        fuzzInputs = [
          pkgs.cargo-fuzz
          rustFmtToolchainTOML
        ];
        fmtInputs = [
          pkgs.alejandra
          pkgs.treefmt
          pkgs.taplo
          pkgs.typos
        ];
        editorConfigInputs = [ pkgs.editorconfig-checker ];
        actionlintInputs = [ pkgs.actionlint ];
        targetDir = "target/${
            pkgs.rust.toRustTarget pkgs.stdenv.targetPlatform
          }/release";
        # Common arguments for the crane build
        commonArgs = {
          inherit
            src
            buildInputs
            nativeBuildInputs
            stdenv
            version
            name
          ;
          pname = name;
        };
        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchainTOML;
        mkExample =
          { example, ... }:
          craneLib.buildPackage (
            commonArgs
            // {
              inherit cargoArtifacts stdenv;
              pname = example;
              cargoExtraArgs = "--example ${example}";
              # Prevent cargo test and nextest from duplicating tests
              doCheck = false;
            }
          );

        # Build *just* the cargo dependencies, so we can reuse
        # all of that work (e.g. via cachix) when running in CI
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;
      in
      rec {
        devShells = {
          default = (pkgs.mkShell.override { inherit stdenv; }) {
            buildInputs =
              shellInputs ++ fmtInputs ++ devInputs ++ buildInputs ++ nativeBuildInputs;
            inherit name;
            FLK_LOG = "debug";
            RUST_BACKTRACE = true;
            RUSTFLAGS = "-C linker=clang -C link-arg=-fuse-ld=${pkgs.mold}/bin/mold";
          };
          editorConfigShell = pkgs.mkShell { buildInputs = editorConfigInputs; };
          actionlintShell = pkgs.mkShell { buildInputs = actionlintInputs; };
          fmtShell = pkgs.mkShell { buildInputs = fmtInputs; };
          fuzzShell = pkgs.mkShell { buildInputs = fuzzInputs; };
        };
        packages =
          {
            default = packages.crane;
            upstream = (pkgs.makeRustPlatform { inherit cargo rustc; }).buildRustPackage {
              cargoDepsName = name;
              GIT_DATE = gitDate;
              GIT_REV = gitRev;
              doCheck = false;
              ASSET_DIR = "${targetDir}/assets/";
              version = "unstable" + gitDate;
              inherit
                name
                src
                stdenv
                nativeBuildInputs
                buildInputs
                cargoLock
              ;
            };
            crane = craneLib.buildPackage (
              commonArgs
              // {
                cargoExtraArgs = "-p ${name}";
                GIT_DATE = gitDate;
                GIT_REV = gitRev;
                doCheck = false;
                ASSET_DIR = "${targetDir}/assets/";
                version = "unstable-" + gitDate;
                pname = name;
                inherit name cargoArtifacts stdenv;
              }
            );
            fuzz =
              ((crane.mkLib pkgs).overrideToolchain rustFmtToolchainTOML).buildPackage
                rec {
                  src = ./.;
                  cargoArtifacts =
                    ((crane.mkLib pkgs).overrideToolchain rustFmtToolchainTOML).buildDepsOnly
                      { src = ./.; };
                  __flags = [
                    "--cfg fuzzing"
                    "--cfg fuzzing_repro"
                    "-Cpasses=sancov-module"
                    "-C opt-level=3"
                    "-Cllvm-args=-sanitizer-coverage-level=4"
                    "-Cllvm-args=-sanitizer-coverage-inline-8bit-counters"
                    "-Cllvm-args=-sanitizer-coverage-pc-table"
                    "-Cllvm-args=-sanitizer-coverage-trace-compares"
                    "-Z sanitizer=address"
                    "-Zsanitizer=memory" # memory 1
                    "-Zsanitizer-memory-track-origins" # memory 2
                    "-Cllvm-args=-sanitizer-coverage-stack-depth" # only linux
                    "-Cdebug-assertions"
                  ];
                  buildFlags = __flags;
                  cargoBuildCommand = "cargo b --package=nix-uri-fuzz --bin fuzz_comp_err";
                  # cargoBuildCommand = "cargo fuzz build fuzz_comp_err";
                  # buildPhaseCargoCommand = "cargo fuzz build fuzz_comp_err";
                  CARGO_PROFILE = "fuzz";
                  GIT_DATE = gitDate;
                  GIT_REV = gitRev;
                  doCheck = false;
                  version = "unstable-" + gitDate;
                  pname = "fuzz_comp_err";
                  nativeBuildInputs = fuzzInputs;
                };
          }
          // pkgs.lib.genAttrs [ "cli" ] (
            example: mkExample { inherit example cargoArtifacts craneLib; }
          );

        apps.default = {
          type = "app";
          program = "${packages.default}/bin/${name}";
        };
        formatter = pkgs.alejandra;
        nixosModules.fuzz-cli = pkgs.nixosTest {
          name = "fuzz-cli";
          nodes.machine =
            { ... }:
            {
              imports = [ {
                environment.systemPackages = [ self.outputs.packages.x86_64-linux.fuzz ];
                virtualisation.graphics = false;
                documentation.enable = false;
              } ];
            };
          testScript = ''
            start_all()
            with subtest("fuzzing"):
                stdout = machine.succeed("fuzz_comp_err", timeout=None)
                print(stdout)
            machine.shutdown()
          '';
        };
      }
    );
}
