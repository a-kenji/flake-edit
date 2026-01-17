{
  perSystem =
    { self', ... }:
    {
      packages = {
        inherit (self'.checks) flake-edit;
        default = self'.checks.flake-edit;
      };
    };
}
