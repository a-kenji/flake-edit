{
  description = "Test flake with a follows declaration the lockfile didn't apply.";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    crane.url = "github:ipetkov/crane";
    # User added this declaration but never ran `nix flake lock`. The lock
    # still resolves crane.nixpkgs to crane's own bundled nixpkgs node.
    crane.inputs.nixpkgs.follows = "nixpkgs";
    # Stale follows: crane no longer has a nested input named `gone`. Triggers
    # the auto-follow removal path so validate_full runs.
    crane.inputs.gone.follows = "nixpkgs";
  };

  outputs =
    {
      self,
      nixpkgs,
      crane,
    }:
    { };
}
