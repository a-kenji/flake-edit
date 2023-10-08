# `$ fe` - edit your flake inputs with ease.
[![Built with Nix](https://img.shields.io/static/v1?label=built%20with&message=nix&color=5277C3&logo=nixos&style=flat-square&logoColor=ffffff)](https://builtwithnix.org)
[![Crates](https://img.shields.io/crates/v/flake-edit?style=flat-square)](https://crates.io/crates/flake-edit)
[![Documentation](https://img.shields.io/badge/flake_edit-documentation-fc0060?style=flat-square)](https://docs.rs/flake-edit)
[![Matrix Chat Room](https://img.shields.io/badge/chat-on%20matrix-1d7e64?logo=matrix&style=flat-square)](https://matrix.to/#/#flake-edit:matrix.org)

Manipulate the inputs of your flake.
Provides a cli application `fe`.
And a library: `flake-edit`.
```
```

## Cli usage

The cli interface `fe` has the following interface:
`$ fe help`
```
Usage: fe [OPTIONS] [FLAKE_REF] <COMMAND>

Commands:
  add
          Add a new flake reference
  pin
          Pin a specific flake reference based on its id
  change
          Pin a specific flake reference based on its id
  remove
          Remove a specific flake reference, based on its id
  list
          List flake inputs
  help
          Print this message or the help of the given subcommand(s)

Arguments:
  [FLAKE_REF]
          

Options:
      --health
          Checks for potential errors in the setup
      --ref-or-rev <REF_OR_REV>
          Pin to a specific ref_or_rev
  -h, --help
          Print help
  -V, --version
          Print version
```

## Examples:
