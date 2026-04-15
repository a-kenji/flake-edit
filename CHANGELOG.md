# Changelog

All notable changes to this project will be documented in this file.

## [0.3.5] - 2026-04-15

### 🐛 Bug Fixes

- *(follow)* Support block-style top-level input follow

### 💼 Other

- Filter interactive menu

### ⚙️ Miscellaneous Tasks

- CI: init an all check

## [0.3.4] - 2026-03-18

### 🚀 Features

- *(config)* Make config flag global
- Use `replace_with` for natural root propagation

### 🐛 Bug Fixes

- Update `ParseError` import path for rnix `0.13.0`
- Append new inputs at the end
- Remove toplevel whitespace

### 💼 Other

- Change `projectRootFile`
- Fix whitespace in trailing slashes
- Try to preserve grouping
- Fix insertion index for @-binding patterns
- Communicate no-op to the user
- Remove leading blank line
- Insert into empty `inputs = { }` block
- Support multi-line output style
- Adjust for `NODE_PAT_ENTRY` for trailing comma detection
- Fix double comma when leading-comma style has trailing comma
- Add transitive dependencies to the top-level
- Fix removal of first entry in leading-comma
- Properly respect `--lock-file`
- Normalize quoted follow attributes
- Handle deeply nested inputs
- Support removal of deeply nested follows
- Skip inputs without url
- Handle spacing
- Add `--transitive` flag

## [0.3.3] - 2026-01-19

### 🚀 Features

- *(follow)* Split into follow and add-follow subcommands

### ⚙️ Miscellaneous Tasks

- *(build)* Improve binary size


## [0.3.2] - 2026-01-19

### 🚀 Features

- Initialize a configuration file
- Unfollow stale follows

### 🐛 Bug Fixes

- Only show diff before commit


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
