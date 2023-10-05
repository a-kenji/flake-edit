# nix-uri
[![Crates](https://img.shields.io/crates/v/nix-uri?style=flat-square)](https://crates.io/crates/nix-uri)
[![Documentation](https://img.shields.io/badge/nix_uri-documentation-fc0060?style=flat-square)](https://docs.rs/nix-uri)


<!-- cargo-rdme start -->

[nix-uri](https://crates.io/crates/nix-uri) is a rust crate that parses
the [nix-uri-scheme](https://nixos.org/manual/nix/stable/command-ref/new-cli/nix3-flake#url-like-syntax)
into a [`FlakeRef`](flakeref::FlakeRef) struct.

Also allows for building a `nix-uri` through the [`FlakeRef`](flakeref::FlakeRef) struct.

Convenience functionality for working with nix `flake.nix` references (flakerefs).
Provides types for the generic attribute set representation, but does not parse it:

```rust
   {
     type = "github";
     owner = "NixOS";
     repo = "nixpkgs";
   }
```

### Installation

To use `nix-uri`, add it as a dependency in your `Cargo.toml` file:

```rust
[dependencies]
nix-uri = "0.1.4"
```

or use `cargo add`:

```rust
cargo add nix-uri
```

## Examples
Check out the examples directory, for more information, or run an example:

```rust
cargo run --example simple
cargo run --example cli github:nixpkgs/nixos
```

The uri syntax representation is parsed by this library:
### Example: Parsing from `github:nixos/nixpkgs`:

 ```rust
  let uri = "github:nixos/nixpkgs";
  let expected = FlakeRef::new(
                FlakeRefType::GitHub {
                owner: "nixos".into(),
                repo: "nixpkgs".into(),
                ref_or_rev: None,
                });
     let parsed: FlakeRef = uri.try_into().unwrap();
     assert_eq!(expected, parsed);
  ```

  It can also be generated from [`FlakeRef`](flakeref::Flakeref).
  ### Example: Constructing the following uri `github:nixos/nixpkgs`:
  ```rust
  let expected = "github:nixos/nixpkgs";
  let uri = FlakeRef::new(
                FlakeRefType::GitHub {
                owner: "nixos".into(),
                repo: "nixpkgs".into(),
                ref_or_rev: None,
                }).to_string();
     assert_eq!(expected, uri);
  ```

<!-- cargo-rdme end -->

# Note 

This library is still an early WIP and not all cases are covered yet.
Especially error handling is not properly implemented at this time.
