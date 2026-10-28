{
  description = "Input app overrides the url of its own nested input `foo` with a top-level flat `inputs.app.inputs.foo.url` declaration.";

  inputs.nixpkgs.url = "github:nixos/nixpkgs";
  inputs.app.url = "github:o/app";
  inputs.app.inputs.foo.url = "github:o/foo";

  outputs = { self, ... }: { };
}
