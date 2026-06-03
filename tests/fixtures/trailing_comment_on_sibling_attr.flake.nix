{
  inputs = {
    dep = {
      url = "github:owner/dep";
      flake = false; # just data, not a flake
    };
  };
  outputs = { self, ... }: { };
}
