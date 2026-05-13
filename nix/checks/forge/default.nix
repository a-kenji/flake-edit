{ lib, ... }:
{
  perSystem =
    { pkgs, self', ... }:
    let
      forgeHelpers = pkgs.python3.pkgs.buildPythonPackage {
        pname = "forge";
        version = "0";
        src = ./.;
        pyproject = true;
        build-system = [ pkgs.python3.pkgs.setuptools ];
      };
    in
    {
      checks = lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
        forge = pkgs.testers.nixosTest {
          name = "forge";

          extraPythonPackages = _: [ forgeHelpers ];

          nodes = {
            forge =
              { config, pkgs, ... }:
              {
                networking.hostName = "forge";
                networking.firewall.allowedTCPPorts = [ 3000 ];

                environment.systemPackages = [
                  config.services.forgejo.package
                  pkgs.curl
                ];

                services.forgejo = {
                  enable = true;

                  # For tests, sqlite keeps setup simple.
                  database.type = "sqlite3";

                  settings = {
                    server = {
                      DOMAIN = "forge";
                      HTTP_PORT = 3000;
                      ROOT_URL = "http://forge:3000/";
                    };
                    service = {
                      DISABLE_REGISTRATION = true;
                    };
                  };
                };
              };

            client =
              { pkgs, ... }:
              {
                environment.systemPackages = with pkgs; [
                  curl
                  nix
                  git
                  self'.packages.flake-edit
                ];

                environment.variables = {
                  FE_LOG = "debug,hyper=off,h2=off";
                  CI = "1";
                };

                nix.settings = {
                  experimental-features = [
                    "nix-command"
                    "flakes"
                  ];
                };
              };
          };

          testScript =
            { nodes, ... }:
            ''
              start_all()
              from forge import run
              run(
                  forge=forge,
                  client=client,
                  forgejo_exe="${nodes.forge.services.forgejo.package}/bin/forgejo",
                  subtest=subtest,
              )
            '';
        };
      };
    };
}
