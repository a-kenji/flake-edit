{
  description = "fixture: a stale follows declaration would otherwise block a valid Z.X -> X candidate via the cycle check. The auto-follow command must remove the stale edge AND emit the unblocked follow in a single invocation.";

  inputs = {
    X.url = "github:owner/X";
    X.inputs.Y.follows = "Y";
    Y.url = "github:owner/Y";
    Y.inputs.Z.follows = "Z";
    Z.url = "github:owner/Z";
  };

  outputs =
    {
      self,
      X,
      Y,
      Z,
    }:
    { };
}
