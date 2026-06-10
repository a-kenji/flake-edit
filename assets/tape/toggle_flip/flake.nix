{
  description = "Edit your flake inputs with ease";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixpkgs-unstable";
    # rust-overlay.url = "github:a-kenji/rust-overlay";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = _: { };
}
