{
  inputs = {
    # nixpkgs.url = "github:nixos/nixpkgs/nixos-25.05";
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    # rust-overlay.url = "path:../rust-overlay";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };
  outputs = _: { };
}
