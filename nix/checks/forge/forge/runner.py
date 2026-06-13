"""Test body for the forgejo integration test. Runs on the host inside `nixos-test-driver`.

This is the actual `testScript` entrypoint.
"""

from __future__ import annotations

import logging
from contextlib import AbstractContextManager
from typing import TYPE_CHECKING, Callable

from forge import _logging
from forge.client import ForgeClient

if TYPE_CHECKING:
    from test_driver.machine import QemuMachine

SubtestFn = Callable[[str], AbstractContextManager[None]]

logger = logging.getLogger("forge.runner")

EMPTY_FLAKE = "{ inputs = { }; outputs = { ... }: { }; }"


def write_empty_flake(machine: QemuMachine, path: str) -> None:
    machine.succeed(f"mkdir -p $(dirname {path}) && echo '{EMPTY_FLAKE}' > {path}")


def write_flake_with_input(
    machine: QemuMachine, path: str, *, input_id: str, url: str
) -> None:
    """Write a flake whose sole input is `input_id`, pinned to `url`."""
    flake = f'{{ inputs.{input_id}.url = "{url}"; outputs = {{ ... }}: {{ }}; }}'
    machine.succeed(f"mkdir -p $(dirname {path}) && echo '{flake}' > {path}")


def run(
    *,
    forge: QemuMachine,
    client: QemuMachine,
    forgejo_exe: str,
    subtest: SubtestFn,
) -> None:
    _logging.configure(level=logging.DEBUG)

    forge.wait_for_unit("forgejo.service")
    forge.wait_for_open_port(3000)

    forge_fc = ForgeClient(forge, base_url="http://localhost:3000")
    client_fc = ForgeClient(client, base_url="http://forge:3000")

    with subtest("Create admin user via CLI"):
        forge.succeed(
            "su -l forgejo -c '"
            "GITEA_WORK_DIR=/var/lib/forgejo; "
            f"{forgejo_exe} --config /var/lib/forgejo/custom/conf/app.ini "
            "admin user create --username test --password totallysafe "
            "--email test@localhost --admin --must-change-password=false'"
        )

    with subtest("Generate API token"):
        api_token = forge_fc.create_token(
            user="test", password="totallysafe", name="token"
        )
        forge_fc.set_token(api_token)
        client_fc.set_token(api_token)

    with subtest("Create repository via API"):
        forge_fc.create_repo(name="project1", auto_init=True)

    with subtest("Verify repository exists"):
        repo = client_fc.get_repo(repo="test/project1")
        assert repo["name"] == "project1"

    with subtest("Create v1.0.0 release"):
        forge_fc.create_release(repo="test/project1", tag="v1.0.0")

    with subtest("Verify v1.0.0 release exists"):
        releases = client_fc.list_releases(repo="test/project1")
        assert len(releases) == 1

    with subtest("Verify public access without API token"):
        anon = ForgeClient(client, base_url="http://forge:3000")
        repo = anon.get_repo(repo="test/project1")
        assert repo["name"] == "project1"
        releases = anon.list_releases(repo="test/project1")
        tags = sorted(r["tag_name"] for r in releases)
        logger.info("Available releases: %s", tags)
        assert "v1.0.0" in tags, "v1.0.0 should be available"

    with subtest("Create empty test flake.nix on client"):
        write_empty_flake(client, "/tmp/test-flake/flake.nix")

    with subtest("Verify empty flake has no inputs"):
        output = client.succeed("cd /tmp/test-flake && flake-edit list")
        logger.info("flake-edit list output (empty): %s", output)
        assert "project1" not in output, "project1 should not be in empty flake"

    with subtest("Add input without version pin"):
        client.succeed(
            "cd /tmp/test-flake && "
            "flake-edit add project1 git+http://forge:3000/test/project1 --no-flake 2>&1"
        )
        flake_content = client.succeed("cat /tmp/test-flake/flake.nix")
        logger.info("flake.nix after add: %s", flake_content)
        assert "project1" in flake_content, "project1 should be added"

    with subtest("Pin input to latest version with update --init"):
        client.succeed("cd /tmp/test-flake && flake-edit update project1 --init 2>&1")
        flake_content = client.succeed("cat /tmp/test-flake/flake.nix")
        logger.info("flake.nix after update --init: %s", flake_content)
        assert "refs/tags/v1.0.0" in flake_content, (
            "Should be pinned to refs/tags/v1.0.0"
        )

    with subtest("Create additional releases (v1.5.0, v2.0.0)"):
        forge_fc.create_release(repo="test/project1", tag="v2.0.0")
        forge_fc.create_release(repo="test/project1", tag="v1.5.0")

    with subtest("Test flake-edit update to latest version"):
        client.succeed("cd /tmp/test-flake && flake-edit update project1 2>&1")
        updated_flake = client.succeed("cat /tmp/test-flake/flake.nix")
        logger.info("Updated flake.nix: %s", updated_flake)
        assert "refs/tags/v2.0.0" in updated_flake, "Should be updated to v2.0.0"
        assert "v1.0.0" not in updated_flake, "Should no longer reference v1.0.0"

    with subtest("Create 'nixos' organization for channel tests"):
        forge_fc.create_org(username="nixos", full_name="NixOS")

    with subtest("Create 'nixpkgs' repository under 'nixos' org"):
        forge_fc.create_org_repo(
            org="nixos",
            name="nixpkgs",
            auto_init=True,
            default_branch="nixos-unstable",
        )

    with subtest("Create channel branches for nixpkgs"):
        default_sha = forge_fc.branch_sha(repo="nixos/nixpkgs", branch="nixos-unstable")
        forge_fc.create_branch(
            repo="nixos/nixpkgs", new="nixos-24.05", base_sha=default_sha
        )

    with subtest("Verify nixpkgs branches exist"):
        branches = client_fc.list_branches(repo="nixos/nixpkgs")
        names = sorted(b["name"] for b in branches)
        logger.info("Available branches: %s", names)
        assert "nixos-24.05" in names, "nixos-24.05 branch should exist"
        assert "nixos-unstable" in names, "nixos-unstable branch should exist"

    with subtest("Create test flake for channel tests"):
        write_empty_flake(client, "/tmp/channel-test/flake.nix")

    with subtest("Add nixpkgs input without version pin"):
        client.succeed(
            "cd /tmp/channel-test && "
            "flake-edit add nixpkgs git+http://forge:3000/nixos/nixpkgs --no-flake 2>&1"
        )
        flake_content = client.succeed("cat /tmp/channel-test/flake.nix")
        logger.info("flake.nix after adding nixpkgs: %s", flake_content)
        assert "nixpkgs" in flake_content, "nixpkgs should be added"
        assert "nixos-24" not in flake_content, (
            "Should not have a channel ref yet (unpinned)"
        )

    with subtest("Channel update --init on unpinned input"):
        client.succeed("cd /tmp/channel-test && flake-edit update nixpkgs --init 2>&1")

    with subtest("Set nixpkgs to nixos-24.05 channel"):
        write_empty_flake(client, "/tmp/channel-test/flake.nix")
        client.succeed(
            "cd /tmp/channel-test && "
            "flake-edit add nixpkgs "
            "'git+http://forge:3000/nixos/nixpkgs?ref=nixos-24.05' --no-flake"
        )

    with subtest("Create nixos-24.11 branch"):
        sha = forge_fc.branch_sha(repo="nixos/nixpkgs", branch="nixos-24.05")
        forge_fc.create_branch(repo="nixos/nixpkgs", new="nixos-24.11", base_sha=sha)
        branches = client_fc.list_branches(repo="nixos/nixpkgs")
        names = sorted(b["name"] for b in branches)
        logger.info("Branches: %s", names)
        assert "nixos-24.11" in names, "nixos-24.11 branch should exist"

    with subtest("Channel update should upgrade from 24.05 to 24.11"):
        client.succeed("cd /tmp/channel-test && flake-edit update nixpkgs 2>&1")
        flake_content = client.succeed("cat /tmp/channel-test/flake.nix")
        logger.info("flake.nix after channel update: %s", flake_content)
        assert "nixos-24.11" in flake_content, "Should be updated to nixos-24.11"
        assert "nixos-24.05" not in flake_content, (
            "Should no longer reference nixos-24.05"
        )

    with subtest("Verify unstable channels are not updated"):
        write_empty_flake(client, "/tmp/channel-test/flake.nix")
        client.succeed(
            "cd /tmp/channel-test && "
            "flake-edit add nixpkgs "
            "'git+http://forge:3000/nixos/nixpkgs?ref=nixos-unstable' --no-flake"
        )
        client.succeed("cd /tmp/channel-test && flake-edit update nixpkgs 2>&1")
        flake_content = client.succeed("cat /tmp/channel-test/flake.nix")
        logger.info("flake.nix after update on unstable: %s", flake_content)
        assert "nixos-unstable" in flake_content, "Should remain on nixos-unstable"
        assert "nixos-24" not in flake_content, (
            "Should NOT be changed to a stable channel"
        )

    with subtest("Verify nixpkgs- prefix channels also work"):
        sha = forge_fc.branch_sha(repo="nixos/nixpkgs", branch="nixos-unstable")
        for new_branch in ("nixpkgs-24.05", "nixpkgs-24.11"):
            forge_fc.create_branch(repo="nixos/nixpkgs", new=new_branch, base_sha=sha)

        write_empty_flake(client, "/tmp/channel-test/flake.nix")
        client.succeed(
            "cd /tmp/channel-test && "
            "flake-edit add nixpkgs "
            "'git+http://forge:3000/nixos/nixpkgs?ref=nixpkgs-24.05' --no-flake"
        )
        client.succeed("cd /tmp/channel-test && flake-edit update nixpkgs 2>&1")
        flake_content = client.succeed("cat /tmp/channel-test/flake.nix")
        logger.info("flake.nix after nixpkgs- update: %s", flake_content)
        assert "nixpkgs-24.11" in flake_content, "Should be updated to nixpkgs-24.11"

    # Tarball-archive inputs (`.../archive/<ref>.tar.gz`) expose no
    # owner/repo to `nix_uri::FlakeRef`, so `update` resolves them on a
    # dedicated path. These subtests cover it end to end.

    with subtest("Create kenji/flake-edit repo with bare YY.MM date-channel branches"):
        forge_fc.create_org(username="kenji", full_name="Kenji")
        forge_fc.create_org_repo(
            org="kenji",
            name="flake-edit",
            auto_init=True,
            default_branch="25.11",
        )
        sha = forge_fc.branch_sha(repo="kenji/flake-edit", branch="25.11")
        forge_fc.create_branch(repo="kenji/flake-edit", new="26.05", base_sha=sha)
        branches = client_fc.list_branches(repo="kenji/flake-edit")
        names = sorted(b["name"] for b in branches)
        logger.info("kenji/flake-edit branches: %s", names)
        assert "25.11" in names and "26.05" in names, "both date branches should exist"

    with subtest("Archive bare YY.MM channel updates 25.11 -> 26.05"):
        write_flake_with_input(
            client,
            "/tmp/archive-channel/flake.nix",
            input_id="flake-edit",
            url="http://forge:3000/kenji/flake-edit/archive/25.11.tar.gz",
        )
        client.succeed("cd /tmp/archive-channel && flake-edit update flake-edit 2>&1")
        flake_content = client.succeed("cat /tmp/archive-channel/flake.nix")
        logger.info("flake.nix after archive channel update: %s", flake_content)
        assert "26.05.tar.gz" in flake_content, "archive ref should be bumped to 26.05"
        assert "25.11" not in flake_content, "should no longer reference 25.11"

    with subtest("Archive semver updates v1.0.0 -> v2.0.0"):
        # Reuse test/project1, which by now carries tags v1.0.0/v1.5.0/v2.0.0.
        write_flake_with_input(
            client,
            "/tmp/archive-semver/flake.nix",
            input_id="project1",
            url="http://forge:3000/test/project1/archive/v1.0.0.tar.gz",
        )
        client.succeed("cd /tmp/archive-semver && flake-edit update project1 2>&1")
        flake_content = client.succeed("cat /tmp/archive-semver/flake.nix")
        logger.info("flake.nix after archive semver update: %s", flake_content)
        assert "v2.0.0.tar.gz" in flake_content, (
            "archive ref should be bumped to v2.0.0"
        )
        assert "v1.0.0" not in flake_content, "should no longer reference v1.0.0"

    with subtest("Archive nixos- channel updates nixos-24.05 -> nixos-24.11"):
        # Reuse nixos/nixpkgs (branches nixos-24.05 / nixos-24.11).
        write_flake_with_input(
            client,
            "/tmp/archive-nixos/flake.nix",
            input_id="nixpkgs",
            url="http://forge:3000/nixos/nixpkgs/archive/nixos-24.05.tar.gz",
        )
        client.succeed("cd /tmp/archive-nixos && flake-edit update nixpkgs 2>&1")
        flake_content = client.succeed("cat /tmp/archive-nixos/flake.nix")
        logger.info("flake.nix after archive nixos channel update: %s", flake_content)
        assert "nixos-24.11.tar.gz" in flake_content, (
            "archive ref should be bumped to nixos-24.11"
        )
        assert "nixos-24.05" not in flake_content, (
            "should no longer reference nixos-24.05"
        )
