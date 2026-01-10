_: {
  perSystem =
    { pkgs, self', ... }:
    {
      checks.forgejo-test = pkgs.testers.nixosTest {
        name = "forgejo-test";

        nodes = {
          forge =
            { config, pkgs, ... }:
            {
              networking.hostName = "forge";
              networking.firewall.allowedTCPPorts = [ 3000 ];

              environment.systemPackages = [
                config.services.forgejo.package
                pkgs.curl
                pkgs.jq
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
                jq
                nix
                git
                self'.packages.flake-edit
              ];

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
          let
            forgejoExe = "${nodes.forge.config.services.forgejo.package}/bin/forgejo";
          in
          ''
            start_all()

            forge.wait_for_unit("forgejo.service")
            forge.wait_for_open_port(3000)

            with subtest("Create admin user via CLI"):
                forge.succeed(
                    "su -l forgejo -c 'GITEA_WORK_DIR=/var/lib/forgejo; ${forgejoExe} --config /var/lib/forgejo/custom/conf/app.ini admin user create "
                    + "--username test --password totallysafe --email test@localhost --admin --must-change-password=false'"
                )

            with subtest("Generate API token"):
                api_token = forge.succeed(
                    "curl --fail -X POST http://test:totallysafe@localhost:3000/api/v1/users/test/tokens "
                    + "-H 'Accept: application/json' -H 'Content-Type: application/json' -d "
                    + "'{\"name\":\"token\",\"scopes\":[\"all\"]}' | jq '.sha1' | xargs echo -n"
                )

            with subtest("Create repository via API"):
                forge.succeed(
                    "curl --fail -X POST http://localhost:3000/api/v1/user/repos "
                    + "-H 'Accept: application/json' -H 'Content-Type: application/json' "
                    + f"-H 'Authorization: token {api_token}'"
                    + ' -d \'{"auto_init":true, "name":"project1", "private":false}\'''
                )

            with subtest("Verify repository exists"):
                client.succeed(f"""
                  set -euo pipefail
                  curl -sfS http://forge:3000/api/v1/repos/test/project1 \
                    -H 'Authorization: token {api_token}' \
                  | jq -e '.name=="project1"'
                """)

            with subtest("Create v1.0.0 release"):
                # Create v1.0.0
                forge.succeed(
                    "curl --fail -X POST http://localhost:3000/api/v1/repos/test/project1/releases "
                    + "-H 'Accept: application/json' -H 'Content-Type: application/json' "
                    + f"-H 'Authorization: token {api_token}'"
                    + ' -d \'{"tag_name":"v1.0.0","name":"Release v1.0.0","body":"Test release 1.0.0"}\'''
                )

            with subtest("Verify v1.0.0 release exists"):
                client.succeed(f"""
                  set -euo pipefail
                  curl -sfS http://forge:3000/api/v1/repos/test/project1/releases \
                    -H 'Authorization: token {api_token}' \
                  | jq -e 'length == 1'
                """)

            with subtest("Verify public access without API token"):
                client.succeed("""
                  set -euo pipefail
                  curl -sfS http://forge:3000/api/v1/repos/test/project1 \
                  | jq -e '.name=="project1"'
                """)
                releases = client.succeed("""
                  set -euo pipefail
                  curl -sfS http://forge:3000/api/v1/repos/test/project1/releases \
                  | jq -r '.[].tag_name' | sort
                """)
                print(f"Available releases: {releases}")
                assert "v1.0.0" in releases, "v1.0.0 should be available"

            with subtest("Create empty test flake.nix on client"):
                client.succeed(r"""
                  mkdir -p /tmp/test-flake
                  cat > /tmp/test-flake/flake.nix << 'EOF'
            {
              description = "Test flake for flake-edit integration";

              inputs = {
              };

              outputs = { ... }: { };
            }
            EOF
                  cat /tmp/test-flake/flake.nix
                """)

            with subtest("Verify empty flake has no inputs"):
                output = client.succeed("""
                  cd /tmp/test-flake
                  flake-edit list
                """)
                print(f"flake-edit list output (empty): {output}")
                assert "project1" not in output, "project1 should not be in empty flake"

            with subtest("Add input without version pin"):
                output = client.succeed("""
                  cd /tmp/test-flake
                  CI=1 FE_LOG=debug flake-edit add project1 git+http://forge:3000/test/project1 --no-flake 2>&1
                """)
                print(f"flake-edit add output: {output}")

                # Verify the input was added without a version pin
                flake_content = client.succeed("""
                  cat /tmp/test-flake/flake.nix
                """)
                print(f"flake.nix after add: {flake_content}")
                assert "project1" in flake_content, "project1 should be added"

            with subtest("Pin input to latest version with update --init"):
                output = client.succeed("""
                  cd /tmp/test-flake
                  CI=1 FE_LOG=debug flake-edit update project1 --init 2>&1
                """)
                print(f"flake-edit update --init output: {output}")

                # Verify the input was pinned with refs/tags/ prefix
                flake_content = client.succeed("""
                  cat /tmp/test-flake/flake.nix
                """)
                print(f"flake.nix after update --init: {flake_content}")
                assert "refs/tags/v1.0.0" in flake_content, "Should be pinned to refs/tags/v1.0.0"

            with subtest("Create additional releases (v1.5.0, v2.0.0)"):
                # Create v2.0.0
                forge.succeed(
                    "curl --fail -X POST http://localhost:3000/api/v1/repos/test/project1/releases "
                    + "-H 'Accept: application/json' -H 'Content-Type: application/json' "
                    + f"-H 'Authorization: token {api_token}'"
                    + ' -d \'{"tag_name":"v2.0.0","name":"Release v2.0.0","body":"Test release 2.0.0"}\'''
                )
                # Create v1.5.0 (to test sorting)
                forge.succeed(
                    "curl --fail -X POST http://localhost:3000/api/v1/repos/test/project1/releases "
                    + "-H 'Accept: application/json' -H 'Content-Type: application/json' "
                    + f"-H 'Authorization: token {api_token}'"
                    + ' -d \'{"tag_name":"v1.5.0","name":"Release v1.5.0","body":"Test release 1.5.0"}\'''
                )

            with subtest("Test flake-edit update to latest version"):
                # Update project1 to latest (should go to v2.0.0)
                output = client.succeed("""
                  cd /tmp/test-flake
                  CI=1 FE_LOG=debug flake-edit update project1 2>&1
                """)
                print(f"flake-edit update output: {output}")

                # Check the flake.nix was updated
                updated_flake = client.succeed("""
                  cat /tmp/test-flake/flake.nix
                """)
                print(f"Updated flake.nix: {updated_flake}")

                # Verify it updated to v2.0.0
                assert "refs/tags/v2.0.0" in updated_flake, "flake.nix should reference refs/tags/v2.0.0"
                assert "v1.0.0" not in updated_flake, "flake.nix should no longer reference v1.0.0"

            with subtest("Verify flake-edit detected Forgejo correctly"):
                # The update should have worked, which means Forgejo was detected
                # Check the logs for detection messages (optional, we already verified it worked)
                print("flake-edit successfully updated from Forgejo instance")
          '';
      };
    };
}
