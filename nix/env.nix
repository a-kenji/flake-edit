{ pkgs, ... }:
{
  RUST_SRC_PATH = pkgs.rustPlatform.rustLibSrc;
}
