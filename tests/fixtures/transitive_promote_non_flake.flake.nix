{
  description = "Two inputs that each pull the same non-flake flake-compat.";

  inputs = {
    colmena.url = "github:zhaofengli/colmena/v0.4.0";
    simple-nixos-mailserver.url = "gitlab:simple-nixos-mailserver/nixos-mailserver/nixos-25.11";
  };

  outputs = _: { };
}
