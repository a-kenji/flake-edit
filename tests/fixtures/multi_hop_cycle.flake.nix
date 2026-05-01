{
  description = "Three top-level inputs whose declared follows form a multi-hop chain (a -> b -> c). A naive per-ancestor cycle predicate misses the case where adding c.a.follows = \"a\" would close the chain back to itself; the cycle detector must catch it.";

  inputs = {
    a.url = "github:example/a";
    b.url = "github:example/b";
    c.url = "github:example/c";

    # Declared multi-hop chain: a's nested b follows top-level b,
    # and b's nested c follows top-level c. If the auto-follow pass
    # were to propose c.a.follows = "a", the chain a -> b -> c -> a
    # would close. The cycle detector must reject that proposal.
    a.inputs.b.follows = "b";
    b.inputs.c.follows = "c";
  };

  outputs =
    {
      self,
      a,
      b,
      c,
    }:
    { };
}
