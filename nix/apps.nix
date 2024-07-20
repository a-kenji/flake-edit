_: {
  perSystem =
    { self', ... }:
    {
      apps = {
        default = self'.packages.default;
        flake-edit = self'.packages.flake-edit;
      };
    };
}
