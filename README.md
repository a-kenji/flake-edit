# `$ flake-edit` - edit your flake inputs with ease
[![Built with Nix](https://img.shields.io/static/v1?label=built%20with&message=nix&color=5277C3&logo=nixos&style=flat-square&logoColor=ffffff)](https://builtwithnix.org)
[![Crates](https://img.shields.io/crates/v/flake-edit?style=flat-square)](https://crates.io/crates/flake-edit)
[![Documentation](https://img.shields.io/badge/flake_edit-documentation-fc0060?style=flat-square)](https://docs.rs/flake-edit)
[![Matrix Chat Room](https://img.shields.io/badge/chat-on%20matrix-1d7e64?logo=matrix&style=flat-square)](https://matrix.to/#/#flake-edit:matrix.org)

`$ flake-edit` - edit your flake inputs with ease.

## `$ flake-edit` - usage

`flake-edit` has the following cli interface:

`$ flake-edit help`

```
Edit your flake inputs with ease

Usage: flake-edit [OPTIONS] [FLAKE_REF] <COMMAND>

Commands:
  add
          Add a new flake reference
  change
          
  remove
          Remove a specific flake reference based on its id
  list
          List flake inputs
  update
          Update inputs to their latest specified release
  help
          Print this message or the help of the given subcommand(s)

Arguments:
  [FLAKE_REF]
          

Options:
      --flake <FLAKE>
          
      --diff
          Print a diff of the changes, will set the apply flag to false
      --apply
          Whether to apply possible changes
  -h, --help
          Print help
  -V, --version
          Print version
```

### `$ flake-edit add`
`$ flake-edit help add`

```
Add a new flake reference

Usage: flake-edit add [OPTIONS] [ID] [URI]

Arguments:
  [ID]
          The name of an input attribute
  [URI]
          The uri that should be added to the input

Options:
      --ref-or-rev <REF_OR_REV>
          Pin to a specific ref_or_rev
  -n, --no-flake
          The input itself is not a flake
  -h, --help
          Print help
```
For some types, the id will be automatically inferred.
![flake-edit add example](https://vhs.charm.sh/vhs-iJiVTOvSd8V9WEl79Ie68.gif)

For some inputs, the uri can be put in directly and the id and type will be inferred.
![flake-edit add inferred example](https://vhs.charm.sh/vhs-3RsaCQO9CAznelPup2kDgV.gif
)

### `$ flake-edit remove`
`$ flake-edit help remove`

```
Remove a specific flake reference based on its id

Usage: flake-edit remove [ID]

Arguments:
  [ID]
          

Options:
  -h, --help
          Print help
```
![flake-edit remove example](https://vhs.charm.sh/vhs-1Uo70AaoEMuYh2UR1JVARD.gif)

### `$ flake-edit update`
`$ flake-edit help update`

```
Update inputs to their latest specified release

Usage: flake-edit update

Options:
  -h, --help
          Print help
```

![flake-edit update example](https://vhs.charm.sh/vhs-5o8mNQOSkW6ZX03fm4yS6q.gif)

### `$ flake-edit list`
`$ flake-edit help list`

```
List flake inputs

Usage: flake-edit list [OPTIONS]

Options:
      --format <FORMAT>
          [default: detailed]
  -h, --help
          Print help
```
List the outputs, that are specified inside the inputs attribute.
![flake-edit list example](https://vhs.charm.sh/vhs-2ZSgdhkzBe3eoxuYtM1JL6.gif)
List the outputs, that are specified inside the inputs attribute, in json format.
![flake-edit list example](https://vhs.charm.sh/vhs-35E6eiL63lFTSC70rQyE1Y.gif)


## License
MIT
