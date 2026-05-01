{
  description = "No follows declared in flake.nix; the cycle exists only in the lockfile's resolved follows. Verifies that lock-only cycles are detected.";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    foo.url = "github:example/foo";
    bar.url = "github:example/bar";
  };

  outputs =
    {
      self,
      nixpkgs,
      foo,
      bar,
    }:
    { };
}
