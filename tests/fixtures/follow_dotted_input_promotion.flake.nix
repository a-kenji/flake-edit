{
  description = "Flake with a top-level input whose flat name contains dots, used as a dedup target.";

  inputs = {
    "ghc-8.6.5-iohk".url = "github:input-output-hk/ghc";
    crane.url = "github:ipetkov/crane";
  };

  outputs =
    {
      self,
      crane,
      ...
    }:
    { };
}
