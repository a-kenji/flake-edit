# Toggle Functionality Implementation

## Overview

This document outlines the implementation of toggle functionality for `flake-edit`, which allows users to switch between commented and uncommented versions of flake inputs.

## Feature Description

The `toggle` command enables users to comment/uncomment specific flake inputs, particularly useful for switching between different versions of the same input (e.g., different rust-overlay sources).

### Usage

```bash
flake-edit toggle [input-id]
flake-edit t [input-id]  # alias
```

### Examples

```bash
# Auto-detect and toggle (when only one toggleable input exists)
flake-edit toggle

# Toggle specific input between commented and uncommented versions
flake-edit toggle rust-overlay
```

### Auto-Detection Behavior

When no `input-id` is specified, flake-edit will:
- **Single toggleable input**: Automatically detect and toggle it
- **Multiple toggleable inputs**: Show error with list of available options
- **No toggleable inputs**: Show error with usage example

### Interactive Selection

When a specified input has multiple versions (multiple commented versions), flake-edit will:
- **Interactive mode (default)**: Prompt user to select which version to activate
- **Non-interactive mode (--non-interactive)**: Show error with available options

### Usage Modes

```bash
# Interactive mode (default) - prompts for selection when multiple versions exist
flake-edit toggle rust-overlay

# Non-interactive mode - errors instead of prompting  
flake-edit --non-interactive toggle rust-overlay
```

## Implementation Details

### 1. Data Structure Changes

#### `src/change.rs`
- Added `Toggle { id: Option<String> }` variant to the `Change` enum
- Added `ToggleToVersion { id: String, target_url: String }` variant for targeted version switching
- Updated `id()` method to handle both toggle variants
- Added `is_toggle()` helper method

### 2. CLI Interface

#### `src/bin/flake-edit/cli.rs`
- Added `Toggle` command to the `Command` enum
- Configured with alias `t` for quick access
- Takes an optional `id` parameter for auto-detection
- Added `--non-interactive` global flag to disable interactive prompts

#### `src/bin/flake-edit/main.rs`
- Added handling for `Command::Toggle` in the main match statement
- Implements interactive vs non-interactive selection logic
- Added `prompt_version_selection()` function for interactive prompts
- Creates `Change::ToggleToVersion` for targeted version switching
- Added `handle_toggle_error()` function for comprehensive error handling
- Provides actionable error messages for different failure scenarios

### 3. Core Logic Implementation

#### `src/edit.rs`
- Added `Change::Toggle` case to `apply_change()` method
- Added `Change::ToggleToVersion` case for targeted version switching
- Delegates to walker's toggle functionality

#### `src/walk.rs`
- Updated `walk()` method to handle both `Change::Toggle` and `Change::ToggleToVersion`
- Added `handle_toggle()` method with auto-detection logic
- Added `handle_toggle_to_version()` method for targeted version switching
- Added `find_toggleable_inputs()` method for input discovery
- Added `toggle_input()` method for multi-version scenario detection
- Added `simple_toggle()` method for basic two-version toggling
- Added `get_input_versions()` method for version extraction
- Toggle logic operates on raw text rather than AST to handle comments

#### `src/error.rs`
- Added `NoToggleableInputs` error for when no toggleable inputs exist
- Added `MultipleToggleableInputs` error with actionable guidance
- Added `NoToggleableVersions` error for inputs without pairs

### 4. Toggle Algorithm

The toggle functionality works at the text level because comments are not preserved in the AST:

1. **Auto-Detection Phase** (when no id specified):
   - Scan all lines to identify toggleable inputs (those with both commented and uncommented versions)
   - Return single input if only one found
   - Error if none or multiple found

2. **Validation Phase**: Ensure target input has both:
   - Active lines: `input-id.url = "..."`
   - Commented lines: `# input-id.url = "..."`

3. **Toggle Phase**: For each line:
   - Comment out active lines by prepending `# `
   - Uncomment commented lines by removing `# ` prefix
   - Handle related lines (e.g., `.inputs.nixpkgs.follows`)
   - Preserve indentation

### 5. Test Coverage

#### Test Fixtures
- `tests/fixtures/toggle_test.flake.nix`: Test fixture with mixed commented/uncommented rust-overlay entries
- `tests/fixtures/toggle_test.flake.lock`: Corresponding lock file

#### Test Implementation
- `tests/edit.rs`: Added `toggle_test_rust_overlay()` test function for specific input toggling
- `tests/edit.rs`: Added `toggle_test_auto_detect()` test function for auto-detection behavior
- Uses `insta` snapshot testing to verify correct toggle behavior

## Files Modified

1. `src/change.rs` - Data structure changes with optional id
2. `src/bin/flake-edit/cli.rs` - CLI command definition with optional parameter
3. `src/bin/flake-edit/main.rs` - CLI command handling and error management
4. `src/edit.rs` - Integration with editing system
5. `src/walk.rs` - Core toggle implementation with auto-detection
6. `src/error.rs` - New error types for toggle scenarios
7. `tests/edit.rs` - Test implementation for both manual and auto-detection
8. `tests/fixtures/toggle_test.flake.nix` - Test fixture
9. `tests/fixtures/toggle_test.flake.lock` - Test fixture

## Key Design Decisions

### Text-Level Manipulation
Comments are not preserved in the AST, so toggle functionality operates directly on the raw text. This ensures:
- Comments are properly handled
- Indentation is preserved
- Original formatting is maintained

### Comprehensive Toggling
The toggle command handles not just the main URL line but also related lines like `.inputs.nixpkgs.follows` to ensure consistent state.

### Auto-Detection Logic
When no input id is specified, the system scans for inputs that have both commented and uncommented versions, enabling seamless toggling without requiring the user to remember input names.

### Comprehensive Error Handling
The implementation provides specific error messages for different scenarios:
- No toggleable inputs found
- Multiple toggleable inputs available (with list)
- Specified input has no toggleable versions

### Safety Checks
The implementation includes safety checks to ensure both versions (commented and uncommented) exist before attempting to toggle, preventing unexpected behavior.

## Testing

Run the toggle-specific tests:
```bash
cargo test toggle_test
```

Run all tests:
```bash
cargo test
```

## Example Behavior

### Simple Toggle (Two Versions)
```nix
# Before
inputs = {
  nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
  # rust-overlay.url = "github:a-kenji/rust-overlay";
  rust-overlay.url = "github:oxalica/rust-overlay";
  rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
};

# After `flake-edit toggle` (auto-detection)
inputs = {
  nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
  rust-overlay.url = "github:a-kenji/rust-overlay";
  # rust-overlay.url = "github:oxalica/rust-overlay";
  # rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
};
```

### Interactive Selection (Multiple Versions)
```nix
# Before
inputs = {
  nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
  # rust-overlay.url = "github:mic92/rust-overlay";
  # rust-overlay.url = "github:a-kenji/rust-overlay"; 
  rust-overlay.url = "github:oxalica/rust-overlay";
  rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
};

# Interactive prompt when running `flake-edit toggle rust-overlay`:
Multiple versions of 'rust-overlay' found:

  1: github:mic92/rust-overlay (commented)
  2: github:a-kenji/rust-overlay (commented)  
  3: github:oxalica/rust-overlay (currently active)

Select which version to activate (1-3): 1

# After selecting option 1:
inputs = {
  nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
  rust-overlay.url = "github:mic92/rust-overlay";
  # rust-overlay.url = "github:a-kenji/rust-overlay";
  # rust-overlay.url = "github:oxalica/rust-overlay";
  # rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
};
```

### Error Examples

#### Multiple toggleable inputs:
```
$ flake-edit toggle
Error: Multiple toggleable inputs found: nixpkgs, rust-overlay.

Please specify which input to toggle:
  flake-edit toggle <input-id>

Available toggleable inputs: nixpkgs, rust-overlay
```

#### No toggleable inputs:
```
$ flake-edit toggle  
Error: No toggleable inputs found.

Toggling requires at least one commented/uncommented pair of inputs in the flake.nix file.
Example:
  nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
  # nixpkgs.url = "github:nixos/nixpkgs/nixos-24.05";
```

#### Non-interactive mode with multiple versions:
```
$ flake-edit --non-interactive toggle rust-overlay
Error: Multiple versions of 'rust-overlay' found.

Please select which version to activate:
  1: github:mic92/rust-overlay
  2: github:a-kenji/rust-overlay
  3: github:oxalica/rust-overlay (currently active)
```

## Future Enhancements

Potential improvements could include:
- Support for toggling multiple inputs at once (`flake-edit toggle --all`)
- More sophisticated comment detection (different comment styles)
- Integration with other flake-edit commands (e.g., `flake-edit add --toggle`)
- Smart suggestions when user provides partial input names
- Configuration options for default toggle behavior