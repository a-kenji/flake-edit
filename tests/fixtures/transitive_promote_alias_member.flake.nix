{
  description = "An input whose nixpkgs and whose flake-parts nixpkgs-lib (nix-community/nixpkgs.lib) are separate lock nodes.";

  inputs = {
    zed.url = "github:zed-industries/zed/v1.26.2-pre";
  };

  outputs = _: { };
}
