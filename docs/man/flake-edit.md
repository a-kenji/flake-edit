# Examples

Add a new flake input:

> ```
> flake-edit add nixpkgs github:NixOS/nixpkgs
> ```

Add an input with automatic ID inference:

> ```
> flake-edit add github:nix-community/home-manager
> ```

Remove a flake input:

> ```
> flake-edit remove nixpkgs
> ```

List all current inputs:

> ```
> flake-edit list
> ```

Update all inputs to latest versions:

> ```
> flake-edit update
> ```

Pin an input to its current revision:

> ```
> flake-edit pin nixpkgs
> ```

Toggle an input between its active url and a stored alternate:

> ```
> flake-edit toggle rust-overlay
> ```

Switch an input to a local checkout, storing the previous url as a
commented alternate:

> ```
> flake-edit toggle ../rust-overlay
> ```

Remove a stored variant's line and flip to the alternative first:

> ```
> flake-edit toggle --remove ../rust-overlay
> ```

Preview changes without applying them:

> ```
> flake-edit --diff add home-manager github:nix-community/home-manager
> ```

Add input without updating the lockfile:

> ```
> flake-edit --no-lock add nixos-hardware github:NixOS/nixos-hardware
> ```
