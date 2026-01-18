# Changelog

All notable changes to this project will be documented in this file.

## [0.3.1] - 2026-01-18

### 🚀 Features

- Init `change` subcommand
- *(cli)* Add --shallow flag to add shallow inputs
- Init `follow` subcommand
- Add `--auto` flag to the follow subcommand
- An interactive mode for querying user input
- Implement nested follows in lock files

### 🐛 Bug Fixes

- *(command)* Improve URI validation
- *(remove)* Correctly also remove orphaned follow inputs
- *(test)* Remove deprecated nodes.*.config usage
- *(tests)* Filter environment-dependent paths in CLI snapshot metadata
- Fix transitive follows in `--auto`

### 📚 Documentation

- Add `follow` subcommand docs

### ⚙️ Miscellaneous Tasks

- Upgrade to Rust 2024 edition
- Improve validation before change
- Parametrize snapshot tests


## [0.3.0] - 2026-11-01

### 🚀 Features

- Keep `flake.lock` in sync with the `flake.nix` file
- Add more exhaustive default completion types
- Init `unpin` subcommand
- Init `gitea` + `forgejo` support for update
- Support channel based releases

### 🐛 Bug Fixes

- *(lib)* Feature gate asset generation for the binary
- Add error on adding duplicate inputs node

### 📚 Documentation

- Add examples to the manpage
- Add `unpin` documentation
- Update `nix-uri` -> `0.1.10`

### CI

- Add auto-merge workflow for dependency updates
- Undeprecate magic-nix-cache-action


## [0.0.2] - 2024-11-04

### 🐛 Bug Fixes

- Adjust outputs with trailing slashes correctly

### ⚙️ Miscellaneous Tasks

- Fix default build

### Update

- Print context messages
- Rename to change_input_to_rev
- Allow any string prefix until the first `-` for semver
- Fix github api authorization

### Devshells

- Add rustc path directly


## [0.1.0] - 2024-09-04
- Initial release
