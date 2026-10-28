{
  description = "Input app overrides the url of its own nested input `foo` with a flat `app.inputs.foo.url` declaration.";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
    app.url = "github:o/app";
    app.inputs.foo.url = "github:o/foo";
  };

  outputs = { self, ... }: { };
}
