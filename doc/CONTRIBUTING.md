# Contributing
Thank you for considering to contribute.
You are invited to contribute new features, fixes or updates, large or small.
Please raise an issue, to see if a potential feature is in scope of the project.
We are always happy to receive contributions and attempt to process them in a timely manner.

## Issues
To get an overview of what can be worked on, please take a look at the [issues](https://github.com/a-kenji/flake-edit/issues?q=is%3Aissue+is%3Aopen+sort%3Aupdated-desc).
If you are unsure if your contribution fits please reach out for synchronous configuration in the [matrix chat room](https://matrix.to/#/#flake-edit:matrix.org).

## How to get tools 
For your convenience you only need one tool to contribute to `flake-edit` or `flake-edit`: `nix`.
You can drop into a development shell with:
```
nix develop
```
or use `direnv`:
```
cat .envrc && direnv allow
```
## Insta
We use `insta` for snapshot testing.
Failing snapshot tests can be reviewed with:
```
cargo insta review
```

## Running Benchmarks

The benchmarks can be run with: 

```
cargo bench
```

Please ensure that your machine is in a stable state and not under heavy load when running the benchmarks for accurate and consistent results.

## Cargo.lock

If a dependency is changed, please do remember to update the lock file accordingly.


# References
- [nix flake](https://nixos.org/manual/nix/unstable/command-ref/new-cli/nix3-flake.html)
