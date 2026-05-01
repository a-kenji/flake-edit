{
  description = "flake.nix declares follows for nested inputs that do not exist in the lockfile. Exercises stale-edge detection.";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    foo.url = "github:example/foo";

    # foo no longer has nested inputs `bar` or `baz`; both follows are stale.
    foo.inputs.bar.follows = "nixpkgs";
    foo.inputs.baz.follows = "nixpkgs";
  };

  outputs =
    {
      self,
      nixpkgs,
      foo,
    }:
    { };
}
