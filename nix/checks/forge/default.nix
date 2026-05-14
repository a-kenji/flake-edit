{ lib, ... }:
{
  perSystem =
    { pkgs, self', ... }:
    let
      forge = pkgs.python3.pkgs.buildPythonPackage {
        name = "forge";
        src = ./.;
        pyproject = true;
        build-system = [ pkgs.python3.pkgs.setuptools ];
        pythonImportsCheck = [
          "forge"
          "forge.api"
          "forge.cli"
          "forge.client"
          "forge.runner"
          "forge.schemas"
        ];
      };
    in
    {
      checks = lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
        forge = pkgs.testers.nixosTest {
          name = "forge";

          extraPythonPackages = _: [ forge ];

          nodes = {
            forge =
              { config, ... }:
              {
                networking.hostName = "forge";
                networking.firewall.allowedTCPPorts = [ 3000 ];

                environment.systemPackages = [
                  config.services.forgejo.package
                  forge
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
                  nix
                  git
                  self'.packages.flake-edit
                  forge
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
              from forge.runner import run
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
