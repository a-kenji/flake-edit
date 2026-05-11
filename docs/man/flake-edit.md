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

Preview changes without applying them:

> ```
> flake-edit --diff add home-manager github:nix-community/home-manager
> ```

Add input without updating lockfile:

> ```
> flake-edit --no-lock add nixos-hardware github:NixOS/nixos-hardware
> ```
