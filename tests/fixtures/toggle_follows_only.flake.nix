{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
    # crane.url = "github:a-kenji/crane";
    crane.url = "github:ipetkov/crane";
    nixpkgs-lib.follows = "nixpkgs";
  };
  outputs = _: { };
}
