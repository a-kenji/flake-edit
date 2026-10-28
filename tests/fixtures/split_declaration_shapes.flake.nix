{
  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    nixpkgs-lib.url = "github:nix-community/nixpkgs.lib";
    g.url = "github:o/g";
    atuin = {
      url = "github:atuinsh/atuin";
      inputs = {
        flake-parts = {
          follows = "flake-parts";
          inputs.nixpkgs-lib.follows = "nixpkgs-lib";
        };
      };
    };
    a = {
      inputs = { t = { follows = ""; }; };
      url = "github:o/a";
    };
    b = {
      url = "github:o/b";
      inputs = {
        n.inputs = { t.follows = ""; };
        x.inputs.g.follows = "g";
      };
    };
    disko = { type = "github"; owner = "nix-community"; repo = "disko"; };
  };
  outputs = _: { };
}
