{
  description = "Test quoted input names with dots";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    "hls-1.10" = { url = "github:haskell/haskell-language-server/1.10.0.0"; flake = false; };
    "hls-2.0" = { url = "github:haskell/haskell-language-server/2.0.0.1"; flake = false; };
    crane = {
      url = "github:ipetkov/crane";
    };
  };

  outputs = { self, nixpkgs, crane, ... }: { };
}
