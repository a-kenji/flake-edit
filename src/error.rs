use thiserror::Error;

#[derive(Debug, Error)]
pub enum FlakeEditError {
    #[error("IoError: {0}")]
    Io(#[from] std::io::Error),
    #[error("The flake should be a root.")]
    NotARoot,
    #[error("There is an error in the Lockfile: {0}")]
    LockError(String),
    #[error("Deserialization Error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error(
        "Input '{0}' already exists in the flake.\n\nTo replace it:\n  1. Remove it first: flake-edit remove {0}\n  2. Then add it again: flake-edit add {0} <flakeref>\n\nOr add it with a different [ID]:\n  flake-edit add [ID] <flakeref>\n\nTo see all current inputs: flake-edit list"
    )]
    DuplicateInput(String),
    #[error(
        "No toggleable inputs found.\n\nToggling requires at least one commented/uncommented pair of inputs in the flake.nix file.\nExample:\n  nixpkgs.url = \"github:nixos/nixpkgs/nixos-unstable\";\n  # nixpkgs.url = \"github:nixos/nixpkgs/nixos-24.05\";"
    )]
    NoToggleableInputs,
    #[error(
        "Multiple toggleable inputs found: {0}.\n\nPlease specify which input to toggle:\n  flake-edit toggle <input-id>\n\nAvailable toggleable inputs: {0}"
    )]
    MultipleToggleableInputs(String),
    #[error(
        "Input '{0}' has no toggleable versions.\n\nToggling requires both commented and uncommented versions of the same input.\nExample for '{0}':\n  {0}.url = \"github:some/repo\";\n  # {0}.url = \"github:other/repo\";"
    )]
    NoToggleableVersions(String),
    #[error("Multiple versions of '{0}' found.\n\nPlease select which version to activate:\n{1}")]
    MultipleVersionsNeedSelection(String, String),
}
