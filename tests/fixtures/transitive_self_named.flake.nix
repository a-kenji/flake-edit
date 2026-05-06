{
  description = "self-named transitive: lock has parent.parent.leaf, declared inside parent block";

  inputs = {
    systems.url = "github:nix-systems/default";
    agenix = {
      url = "github:ryantm/agenix";
      inputs.agenix.inputs.systems.follows = "systems";
    };
  };

  outputs =
    {
      self,
      systems,
      agenix,
    }:
    { };
}
