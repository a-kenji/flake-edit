{ self, rustPlatform, installShellFiles, lib

}:
let
  cargoTOML = builtins.fromTOML (builtins.readFile (self + "/Cargo.toml"));
  inherit (cargoTOML.package) version name;
  gitDate = "${builtins.substring 0 4 self.lastModifiedDate}-${
      builtins.substring 4 2 self.lastModifiedDate
    }-${builtins.substring 6 2 self.lastModifiedDate}";
  gitRev = self.shortRev or self.dirtyShortRev;
  meta = import ./meta.nix { inherit lib; };
in rustPlatform.buildRustPackage {
  inherit version name meta;
  env = {
    GIT_DATE = gitDate;
    GIT_REV = gitRev;
  };
  src = self;
  cargoLock.lockFile = self + "/Cargo.lock";

  nativeBuildInputs = [ ];

  buildInputs = [ installShellFiles ];
}
