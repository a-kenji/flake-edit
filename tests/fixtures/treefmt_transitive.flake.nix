{
  description = "test transitive follows with treefmt-nix and treefmt";

  inputs = {
    treefmt-nix.url = "github:numtide/treefmt-nix";
    treefmt.url = "github:numtide/treefmt";
  };

  outputs =
    {
      self,
      treefmt-nix,
      treefmt,
    }:
    { };
}
