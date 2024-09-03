{
  self,
  lib,
  pkgs,
  installShellFiles,
  openssl,
  pkg-config,
}:
let
  cargoTOML = builtins.fromTOML (builtins.readFile (self + "/Cargo.toml"));
  inherit (cargoTOML.package) version name;
  pname = name;
  gitDate = "${builtins.substring 0 4 self.lastModifiedDate}-${
    builtins.substring 4 2 self.lastModifiedDate
  }-${builtins.substring 6 2 self.lastModifiedDate}";
  gitRev = self.shortRev or self.dirtyShortRev;
  meta = import ./meta.nix { inherit lib; };
  # crane
  craneLib = self.inputs.crane.mkLib pkgs;
  commonArgs = {
    nativeBuildInputs = [
      pkg-config
      openssl
    ];
    inherit version name pname;
    src = lib.cleanSourceWith { src = craneLib.path ../.; };
  };
  cargoArtifacts = craneLib.buildDepsOnly commonArgs;
  cargoClippy = craneLib.cargoClippy (commonArgs // { inherit cargoArtifacts; });
  cargoDeny = craneLib.cargoDeny (commonArgs // { inherit cargoArtifacts; });
  cargoTarpaulin = craneLib.cargoTarpaulin (commonArgs // { inherit cargoArtifacts; });
  cargoDoc = craneLib.cargoDoc (commonArgs // { inherit cargoArtifacts; });
  cargoTest = craneLib.cargoNextest (commonArgs // { inherit cargoArtifacts; });
  assetDir = "target/assets";
  postInstall = ''
    # install the manpage
    installManPage ${assetDir}/${name}.1
    # explicit behavior
    # cp ${assetDir}/${name}.bash ./completions.bash
    # installShellCompletion --bash --name ${name}.bash ./completions.bash
    # cp ${assetDir}/${name}.fish ./completions.fish
    # installShellCompletion --fish --name ${name}.fish ./completions.fish
    # cp ${assetDir}/_${name} ./completions.zsh
    # installShellCompletion --zsh --name _${name} ./completions.zsh
    # mkdir -p $out/share/nu
    # cp ${assetDir}/${name}.nu $out/share/${name}.nu
      installShellCompletion --cmd ${name} \
    --bash <($out/bin/${name} complete bash) \
    --fish <($out/bin/${name} complete fish) \
    --zsh <($out/bin/${name} complete zsh)
  '';
in
{
  flake-edit = craneLib.buildPackage (
    commonArgs
    // {
      cargoExtraArgs = "-p ${name}";
      buildInputs = [ installShellFiles ];
      env = {
        GIT_DATE = gitDate;
        GIT_REV = gitRev;
        ASSET_DIR = assetDir;
      };
      doCheck = false;
      version = "unstable-" + gitDate;
      inherit name pname assetDir cargoArtifacts meta postInstall;
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
