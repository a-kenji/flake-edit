_: {
  perSystem = { self', ... }: {
    apps = {
      default = self'.packages.default;
      fe = self'.packages.fe;
    };
  };
}
