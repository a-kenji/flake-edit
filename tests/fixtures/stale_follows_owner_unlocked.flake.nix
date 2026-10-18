{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs";
    b.url = "github:o/b";
    sops-nix = {
      url = "github:Mic92/sops-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };
  outputs = _: { };
}
