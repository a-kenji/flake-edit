{
  description = "Multi-hop cycle exercise where one participant is a quoted, dot-named segment (\"hls-1.10\"). Validates that typed structural equality, not URL-prefix string compare, catches the cycle.";

  inputs = {
    "hls-1.10".url = "github:example/hls";
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    helper.url = "github:example/helper";

    # The dotted name participates as both a top-level input and a
    # nested follow target. URL-prefix string compare misses the cycle
    # on the embedded "hls-1.10/" form; structural AttrPath equality
    # catches it.
    helper.inputs.nixpkgs.follows = "nixpkgs";
    "hls-1.10".inputs.helper.follows = "helper";
  };

  outputs =
    {
      self,
      nixpkgs,
      helper,
    }:
    { };
}
