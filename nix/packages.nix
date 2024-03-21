{ self, ... }: {
  perSystem = { pkgs, ... }: {
    packages = {
      default = pkgs.callPackage ./fe.nix { inherit self; };
      fe = pkgs.callPackage ./fe.nix { inherit self; };
    };
  };
}

#     # Common arguments for the crane build
#     commonArgs = {
#       inherit stdenv version name;
#       pname = name;
#       src = pkgs.lib.cleanSourceWith {src = craneLib.path ./.;};
#     };
#     craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchainTOML;
#     # Build *just* the cargo dependencies, so we can reuse
#     # all of that work (e.g. via cachix) when running in CI
#     assetDir = "target/assets";
#     postInstall = name: ''
#       # install the manpage
#       installManPage ${assetDir}/${name}.1
#       # explicit behavior
#       cp ${assetDir}/${name}.bash ./completions.bash
#       installShellCompletion --bash --name ${name}.bash ./completions.bash
#       cp ${assetDir}/${name}.fish ./completions.fish
#       installShellCompletion --fish --name ${name}.fish ./completions.fish
#       cp ${assetDir}/_${name} ./completions.zsh
#       installShellCompletion --zsh --name _${name} ./completions.zsh
#       mkdir -p $out/share/nu
#       cp ${assetDir}/${name}.nu $out/share/${name}.nu
#     '';
#
#     meta = with pkgs.lib; {
#       homepage = "https://github.com/a-kenji/fe";
#       inherit description;
#       mainProgram = "fe";
#       license = [licenses.mit];
#     };
#     cargoArtifacts = craneLib.buildDepsOnly commonArgs;
#     cargoClippy = craneLib.cargoClippy (
#       commonArgs
#       // {
#         inherit cargoArtifacts;
#         nativeBuildInputs = [rustToolchainDevTOML];
#       }
#     );
#     cargoDeny = craneLib.cargoDeny (commonArgs // {inherit cargoArtifacts;});
#     cargoTarpaulin = craneLib.cargoTarpaulin (
#       commonArgs // {inherit cargoArtifacts;}
#     );
#     cargoDoc = craneLib.cargoDoc (commonArgs // {inherit cargoArtifacts;});
#     cargoTest = craneLib.cargoTest (commonArgs // {inherit cargoArtifacts;});
#     cranePackage = craneLib.buildPackage (
#       commonArgs
#       // {
#         cargoExtraArgs = "-p ${name}";
#         GIT_DATE = gitDate;
#         GIT_REV = gitRev;
#         ASSET_DIR = assetDir;
#         doCheck = false;
#         version = "unstable-" + gitDate;
#         pname = "fe";
#         name = "fe";
#         postInstall = postInstall "fe";
#         inherit
#           assetDir
#           buildInputs
#           cargoArtifacts
#           meta
#           stdenv
#           ;
#       }
#     );
#     packages = rec {
#       default = fe;
#       fe = cranePackage;
#       inherit
#         cargoArtifacts
#         cargoClippy
#         cargoDeny
#         cargoDoc
#         cargoTest
#         cargoTarpaulin
#         ;
#     };
# }

