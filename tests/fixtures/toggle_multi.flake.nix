{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    # rust-overlay.url = "github:a-kenji/rust-overlay";
    # rust-overlay.url = "path:../rust-overlay";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };
  outputs = _: { };
}
