# Contributing
Thank you for considering to contribute.
You are invited to contribute new features, fixes or updates, large or small.
We are always happy to receive contributions and attempt to process them in a timely manner.

## Issues
To get an overview of what can be worked on, please take a look at the [issues](https://github.com/a-kenji/nix-uri/issues?q=is%3Aissue+is%3Aopen+sort%3Aupdated-desc).

## How to get tools 
For your convenience you only need one tool to contribute to `nix-uri`: `nix`.
You can drop into a development shell with:
```
nix develop
```
or use `direnv`:
```
cat .envrc && direnv allow
```

If you want to set the environment manually, the rust-toolchain version
that will be assumed is referenced inside `rust-toolchain.toml`.

## Steps
There is a lint target in the `justfile`, that can be run with:
```
just lint
```

The `rustfmt` version is referenced inside the `.rustfmt-toolchain.toml`.
The `clippy` version is referenced inside `rust-toolchain.toml`, only lints targeting that version will be merged.

## Running Benchmarks

The benchmarks can be run with: 

```
cargo bench
```

Please ensure that your machine is in a stable state and not under heavy load when running the benchmarks for accurate and consistent results.

## Cargo.lock

Although `nix-uri` is a library, the Cargo.lock file is included to build the examples more efficiently. 
This inclusion does not affect the library's consumers. 
If a dependency is changed, please do remember to update the lock file accordingly.


# References
- [nix flakes](https://nixos.org/manual/nix/stable/command-ref/new-cli/nix3-flake)
