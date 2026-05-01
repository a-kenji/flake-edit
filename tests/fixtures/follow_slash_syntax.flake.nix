{
  description = "slash-form follows target: `inputs.X.follows = \"a/b\"` is the syntactic alias of dot form";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    parent.url = "github:foo/parent";
    consumer.url = "github:foo/consumer";
    consumer.inputs.child.follows = "parent/child";
  };

  outputs =
    {
      self,
      nixpkgs,
      parent,
      consumer,
    }:
    { };
}
