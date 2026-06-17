{ inputs, lib, ... }:
{
  perSystem =
    { pkgs, ... }:
    {
      checks.inputs = pkgs.linkFarm "inputs" (
        lib.mapAttrsToList (name: input: {
          inherit name;
          path = input.outPath;
        }) (removeAttrs inputs [ "self" ])
      );
    };
}
