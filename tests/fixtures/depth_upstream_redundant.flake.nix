{
  description = "Depth-2 follow already covered by upstream propagation";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-edit.url = "github:a-kenji/flake-edit";
    flake-edit.inputs.nixpkgs.follows = "nixpkgs";
    tui-term.url = "github:a-kenji/tui-term";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-edit,
      tui-term,
    }:
    { };
}
