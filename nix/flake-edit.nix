{
  self,
  lib,
  pkgs,
  installShellFiles,
  openssl,
  pkg-config,
}:
let
  cargoTOML = fromTOML (builtins.readFile (self + "/Cargo.toml"));
  inherit (cargoTOML.package) version name;
  pname = name;
  gitDate = "${builtins.substring 0 4 self.lastModifiedDate}-${
    builtins.substring 4 2 self.lastModifiedDate
  }-${builtins.substring 6 2 self.lastModifiedDate}";
  gitRev = self.shortRev or self.dirtyShortRev;
  meta = import ./meta.nix { inherit lib; };
  # crane
  craneLib = self.inputs.crane.mkLib pkgs;
  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../src
      ../benches
      ../tests
    ];
  };
  commonArgs = {
    nativeBuildInputs = [
      pkg-config
      openssl
    ];
    inherit
      version
      name
      pname
      src
      ;
  };
  cargoArtifacts = craneLib.buildDepsOnly commonArgs;
  cargoClippy = craneLib.cargoClippy (commonArgs // { inherit cargoArtifacts; });
  cargoDeny = craneLib.cargoDeny (commonArgs // { inherit cargoArtifacts; });
  cargoTarpaulin = craneLib.cargoTarpaulin (commonArgs // { inherit cargoArtifacts; });
  cargoDoc = craneLib.cargoDoc (commonArgs // { inherit cargoArtifacts; });
  cargoTest = craneLib.cargoNextest (commonArgs // { inherit cargoArtifacts; });
  # Generate shell completions via CompleteEnv
  postInstall = ''
    COMPLETE=bash $out/bin/${name} > ${name}.bash
    COMPLETE=zsh $out/bin/${name} > _${name}
    COMPLETE=fish $out/bin/${name} > ${name}.fish
    installShellCompletion --bash --name ${name}.bash ${name}.bash
    installShellCompletion --zsh --name _${name} _${name}
    installShellCompletion --fish --name ${name}.fish ${name}.fish
  '';
in
{
  flake-edit = craneLib.buildPackage (
    commonArgs
    // {
      cargoExtraArgs = "-p ${name}";
      nativeBuildInputs = commonArgs.nativeBuildInputs ++ [ installShellFiles ];
      env = {
        GIT_DATE = gitDate;
        GIT_REV = gitRev;
      };
      doCheck = false;
      version = version + "-unstable-" + gitDate;
      inherit
        name
        pname
        postInstall
        cargoArtifacts
        meta
        ;
    }
  );
  inherit
    cargoClippy
    cargoArtifacts
    cargoDeny
    cargoTarpaulin
    cargoDoc
    cargoTest
    ;
}
