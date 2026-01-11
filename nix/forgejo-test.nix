{
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
          let
            forgejoExe = "${nodes.forge.config.services.forgejo.package}/bin/forgejo";
          in
          ''
            EMPTY_FLAKE = "{ inputs = { }; outputs = { ... }: { }; }"

            def write_empty_flake(machine, path):
                machine.succeed(f"mkdir -p $(dirname {path}) && echo '{EMPTY_FLAKE}' > {path}")

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
                write_empty_flake(client, "/tmp/test-flake/flake.nix")
                client.succeed("cat /tmp/test-flake/flake.nix")

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
                  flake-edit add project1 git+http://forge:3000/test/project1 --no-flake 2>&1
                """)
                print(f"flake-edit add output: {output}")

                flake_content = client.succeed("cat /tmp/test-flake/flake.nix")
                print(f"flake.nix after add: {flake_content}")
                assert "project1" in flake_content, "project1 should be added"

            with subtest("Pin input to latest version with update --init"):
                output = client.succeed("""
                  cd /tmp/test-flake
                  flake-edit update project1 --init 2>&1
                """)
                print(f"flake-edit update --init output: {output}")

                flake_content = client.succeed("cat /tmp/test-flake/flake.nix")
                print(f"flake.nix after update --init: {flake_content}")
                assert "refs/tags/v1.0.0" in flake_content, "Should be pinned to refs/tags/v1.0.0"

            with subtest("Create additional releases (v1.5.0, v2.0.0)"):
                forge.succeed(
                    "curl --fail -X POST http://localhost:3000/api/v1/repos/test/project1/releases "
                    + "-H 'Accept: application/json' -H 'Content-Type: application/json' "
                    + f"-H 'Authorization: token {api_token}'"
                    + ' -d \'{"tag_name":"v2.0.0","name":"Release v2.0.0","body":"Test release 2.0.0"}\'''
                )
                forge.succeed(
                    "curl --fail -X POST http://localhost:3000/api/v1/repos/test/project1/releases "
                    + "-H 'Accept: application/json' -H 'Content-Type: application/json' "
                    + f"-H 'Authorization: token {api_token}'"
                    + ' -d \'{"tag_name":"v1.5.0","name":"Release v1.5.0","body":"Test release 1.5.0"}\'''
                )

            with subtest("Test flake-edit update to latest version"):
                output = client.succeed("""
                  cd /tmp/test-flake
                  flake-edit update project1 2>&1
                """)
                print(f"flake-edit update output: {output}")

                updated_flake = client.succeed("cat /tmp/test-flake/flake.nix")
                print(f"Updated flake.nix: {updated_flake}")
                assert "refs/tags/v2.0.0" in updated_flake, "Should be updated to v2.0.0"
                assert "v1.0.0" not in updated_flake, "Should no longer reference v1.0.0"

            with subtest("Verify flake-edit detected Forgejo correctly"):
                print("flake-edit successfully updated from Forgejo instance")

            with subtest("Create 'nixos' organization for channel tests"):
                forge.succeed(
                    "curl --fail -X POST http://localhost:3000/api/v1/orgs "
                    + "-H 'Accept: application/json' -H 'Content-Type: application/json' "
                    + f"-H 'Authorization: token {api_token}'"
                    + ' -d \'{"username":"nixos","full_name":"NixOS","visibility":"public"}\'''
                )

            with subtest("Create 'nixpkgs' repository under 'nixos' org"):
                forge.succeed(
                    "curl --fail -X POST http://localhost:3000/api/v1/orgs/nixos/repos "
                    + "-H 'Accept: application/json' -H 'Content-Type: application/json' "
                    + f"-H 'Authorization: token {api_token}'"
                    + ' -d \'{"auto_init":true, "name":"nixpkgs", "private":false, "default_branch":"nixos-unstable"}\'''
                )

            with subtest("Create channel branches for nixpkgs"):
                default_sha = forge.succeed(
                    "curl -sfS http://localhost:3000/api/v1/repos/nixos/nixpkgs/branches/nixos-unstable "
                    + f"-H 'Authorization: token {api_token}' "
                    + "| jq -r '.commit.id' | xargs echo -n"
                )
                forge.succeed(
                    "curl --fail -X POST http://localhost:3000/api/v1/repos/nixos/nixpkgs/branches "
                    + "-H 'Accept: application/json' -H 'Content-Type: application/json' "
                    + f"-H 'Authorization: token {api_token}' "
                    + f'-d \'{{"new_branch_name":"nixos-24.05","old_ref_name":"{default_sha}"}}\'''
                )

            with subtest("Verify nixpkgs branches exist"):
                branches = client.succeed("""
                  curl -sfS http://forge:3000/api/v1/repos/nixos/nixpkgs/branches \
                  | jq -r '.[].name' | sort
                """)
                print(f"Available branches: {branches}")
                assert "nixos-24.05" in branches, "nixos-24.05 branch should exist"
                assert "nixos-unstable" in branches, "nixos-unstable branch should exist"

            with subtest("Create test flake for channel tests"):
                write_empty_flake(client, "/tmp/channel-test/flake.nix")
                client.succeed("cat /tmp/channel-test/flake.nix")

            with subtest("Add nixpkgs input without version pin"):
                output = client.succeed("""
                  cd /tmp/channel-test
                  flake-edit add nixpkgs git+http://forge:3000/nixos/nixpkgs --no-flake 2>&1
                """)
                print(f"flake-edit add nixpkgs output: {output}")

                flake_content = client.succeed("cat /tmp/channel-test/flake.nix")
                print(f"flake.nix after adding nixpkgs: {flake_content}")
                assert "nixpkgs" in flake_content, "nixpkgs should be added"
                assert "nixos-24" not in flake_content, "Should not have a channel ref yet (unpinned)"

            with subtest("Channel update --init on unpinned input"):
                output = client.succeed("""
                  cd /tmp/channel-test
                  flake-edit update nixpkgs --init 2>&1
                """)
                print(f"flake-edit update --init output: {output}")
                flake_content = client.succeed("cat /tmp/channel-test/flake.nix")
                print(f"flake.nix after update --init: {flake_content}")

            with subtest("Set nixpkgs to nixos-24.05 channel"):
                write_empty_flake(client, "/tmp/channel-test/flake.nix")
                client.succeed("""
                  cd /tmp/channel-test
                  flake-edit add nixpkgs 'git+http://forge:3000/nixos/nixpkgs?ref=nixos-24.05' --no-flake
                """)
                client.succeed("cat /tmp/channel-test/flake.nix")

            with subtest("Create nixos-24.11 branch"):
                branch_sha = forge.succeed(
                    "curl -sfS http://localhost:3000/api/v1/repos/nixos/nixpkgs/branches/nixos-24.05 "
                    + f"-H 'Authorization: token {api_token}' "
                    + "| jq -r '.commit.id' | xargs echo -n"
                )
                forge.succeed(
                    "curl --fail -X POST http://localhost:3000/api/v1/repos/nixos/nixpkgs/branches "
                    + "-H 'Accept: application/json' -H 'Content-Type: application/json' "
                    + f"-H 'Authorization: token {api_token}' "
                    + f'-d \'{{"new_branch_name":"nixos-24.11","old_ref_name":"{branch_sha}"}}\'''
                )
                branches = client.succeed("""
                  curl -sfS http://forge:3000/api/v1/repos/nixos/nixpkgs/branches \
                  | jq -r '.[].name' | sort
                """)
                print(f"Branches: {branches}")
                assert "nixos-24.11" in branches, "nixos-24.11 branch should exist"

            with subtest("Channel update should upgrade from 24.05 to 24.11"):
                output = client.succeed("""
                  cd /tmp/channel-test
                  flake-edit update nixpkgs 2>&1
                """)
                print(f"flake-edit channel update output: {output}")

                flake_content = client.succeed("cat /tmp/channel-test/flake.nix")
                print(f"flake.nix after channel update: {flake_content}")
                assert "nixos-24.11" in flake_content, "Should be updated to nixos-24.11"
                assert "nixos-24.05" not in flake_content, "Should no longer reference nixos-24.05"

            with subtest("Verify unstable channels are not updated"):
                write_empty_flake(client, "/tmp/channel-test/flake.nix")
                client.succeed("""
                  cd /tmp/channel-test
                  flake-edit add nixpkgs 'git+http://forge:3000/nixos/nixpkgs?ref=nixos-unstable' --no-flake
                """)
                output = client.succeed("""
                  cd /tmp/channel-test
                  flake-edit update nixpkgs 2>&1
                """)
                print(f"flake-edit update on unstable output: {output}")

                flake_content = client.succeed("cat /tmp/channel-test/flake.nix")
                print(f"flake.nix after update on unstable: {flake_content}")
                assert "nixos-unstable" in flake_content, "Should remain on nixos-unstable"
                assert "nixos-24" not in flake_content, "Should NOT be changed to a stable channel"

            with subtest("Verify nixpkgs- prefix channels also work"):
                branch_sha = forge.succeed(
                    "curl -sfS http://localhost:3000/api/v1/repos/nixos/nixpkgs/branches/nixos-unstable "
                    + f"-H 'Authorization: token {api_token}' "
                    + "| jq -r '.commit.id' | xargs echo -n"
                )
                forge.succeed(
                    "curl --fail -X POST http://localhost:3000/api/v1/repos/nixos/nixpkgs/branches "
                    + "-H 'Accept: application/json' -H 'Content-Type: application/json' "
                    + f"-H 'Authorization: token {api_token}' "
                    + f'-d \'{{"new_branch_name":"nixpkgs-24.05","old_ref_name":"{branch_sha}"}}\'''
                )
                forge.succeed(
                    "curl --fail -X POST http://localhost:3000/api/v1/repos/nixos/nixpkgs/branches "
                    + "-H 'Accept: application/json' -H 'Content-Type: application/json' "
                    + f"-H 'Authorization: token {api_token}' "
                    + f'-d \'{{"new_branch_name":"nixpkgs-24.11","old_ref_name":"{branch_sha}"}}\'''
                )

                write_empty_flake(client, "/tmp/channel-test/flake.nix")
                client.succeed("""
                  cd /tmp/channel-test
                  flake-edit add nixpkgs 'git+http://forge:3000/nixos/nixpkgs?ref=nixpkgs-24.05' --no-flake
                """)
                output = client.succeed("""
                  cd /tmp/channel-test
                  flake-edit update nixpkgs 2>&1
                """)
                print(f"flake-edit update nixpkgs- prefix output: {output}")

                flake_content = client.succeed("cat /tmp/channel-test/flake.nix")
                print(f"flake.nix after nixpkgs- update: {flake_content}")
                assert "nixpkgs-24.11" in flake_content, "Should be updated to nixpkgs-24.11"
          '';
      };
    };
}
