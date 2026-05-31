{
  description = "Two top-level inputs whose lockfile-resolved nested inputs share a common target, alongside a depth-1 follows declaration whose target is not a top-level input of this flake.";

  inputs = {
    foo.url = "github:example/foo";
    foo.inputs.nixpkgs.follows = "nixpkgs";
    bar.url = "github:example/bar";
  };

  outputs =
    { self, foo, bar }:
    { };
}
