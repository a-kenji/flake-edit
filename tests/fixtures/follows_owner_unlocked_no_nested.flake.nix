{
  inputs = {
    nixpkgs.url = "github:o/nixpkgs";
    c.url = "github:o/c";
    c.inputs.nixpkgs.follows = "nixpkgs";
  };
  outputs = { self, ... }: { };
}
