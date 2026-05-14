"""Entry point for the forgejo NixOS VM integration test.

The Nix-side `testScript` injects the machine objects and `subtest` context
manager, then hands them to `run()`.
"""

from __future__ import annotations

import json
import shlex
from contextlib import AbstractContextManager
from typing import Any, Callable

from test_driver.machine import QemuMachine

JsonValue = Any
SubtestFn = Callable[[str], AbstractContextManager[None]]

EMPTY_FLAKE = "{ inputs = { }; outputs = { ... }: { }; }"


def write_empty_flake(machine: QemuMachine, path: str) -> None:
    machine.succeed(f"mkdir -p $(dirname {path}) && echo '{EMPTY_FLAKE}' > {path}")


def api(
    machine: QemuMachine,
    method: str,
    path: str,
    *,
    host: str = "localhost:3000",
    token: str | None = None,
    basic_auth: tuple[str, str] | None = None,
    body: JsonValue = None,
) -> JsonValue:
    """Issue a JSON HTTP request from inside a VM via curl.

    Body, when given, is sent as JSON via a heredoc to avoid quoting the
    payload three times through shell, Python, and Nix. Response body, when
    non-empty, is parsed as JSON and returned.
    """
    args = [
        "curl",
        "--fail",
        "-sS",
        "-X",
        method,
        "-H",
        "Accept: application/json",
    ]
    if token is not None:
        args += ["-H", f"Authorization: token {token}"]
    url_host = host
    if basic_auth is not None:
        user, password = basic_auth
        url_host = f"{user}:{password}@{host}"
    if body is not None:
        args += [
            "-H",
            "Content-Type: application/json",
            "--data-binary",
            "@-",
        ]
    args.append(f"http://{url_host}{path}")
    cmd = " ".join(shlex.quote(a) for a in args)
    if body is not None:
        stdin = json.dumps(body)
        raw = machine.succeed(f"{cmd} <<'PY_BODY_EOF'\n{stdin}\nPY_BODY_EOF")
    else:
        raw = machine.succeed(cmd)
    return json.loads(raw) if raw.strip() else None


def branch_sha(machine: QemuMachine, repo: str, branch: str, *, token: str) -> str:
    info = api(machine, "GET", f"/api/v1/repos/{repo}/branches/{branch}", token=token)
    return info["commit"]["id"]


def create_release(machine: QemuMachine, repo: str, tag: str, *, token: str) -> None:
    api(
        machine,
        "POST",
        f"/api/v1/repos/{repo}/releases",
        token=token,
        body={"tag_name": tag, "name": f"Release {tag}", "body": f"Test release {tag}"},
    )


def create_branch(
    machine: QemuMachine, repo: str, new: str, *, base_sha: str, token: str
) -> None:
    api(
        machine,
        "POST",
        f"/api/v1/repos/{repo}/branches",
        token=token,
        body={"new_branch_name": new, "old_ref_name": base_sha},
    )


def run(
    *,
    forge: QemuMachine,
    client: QemuMachine,
    forgejo_exe: str,
    subtest: SubtestFn,
) -> None:
    forge.wait_for_unit("forgejo.service")
    forge.wait_for_open_port(3000)

    with subtest("Create admin user via CLI"):
        forge.succeed(
            "su -l forgejo -c '"
            "GITEA_WORK_DIR=/var/lib/forgejo; "
            f"{forgejo_exe} --config /var/lib/forgejo/custom/conf/app.ini "
            "admin user create --username test --password totallysafe "
            "--email test@localhost --admin --must-change-password=false'"
        )

    with subtest("Generate API token"):
        token_resp = api(
            forge,
            "POST",
            "/api/v1/users/test/tokens",
            basic_auth=("test", "totallysafe"),
            body={"name": "token", "scopes": ["all"]},
        )
        api_token: str = token_resp["sha1"]

    with subtest("Create repository via API"):
        api(
            forge,
            "POST",
            "/api/v1/user/repos",
            token=api_token,
            body={"auto_init": True, "name": "project1", "private": False},
        )

    with subtest("Verify repository exists"):
        repo = api(
            client,
            "GET",
            "/api/v1/repos/test/project1",
            host="forge:3000",
            token=api_token,
        )
        assert repo["name"] == "project1"

    with subtest("Create v1.0.0 release"):
        create_release(forge, "test/project1", "v1.0.0", token=api_token)

    with subtest("Verify v1.0.0 release exists"):
        releases = api(
            client,
            "GET",
            "/api/v1/repos/test/project1/releases",
            host="forge:3000",
            token=api_token,
        )
        assert len(releases) == 1

    with subtest("Verify public access without API token"):
        repo = api(client, "GET", "/api/v1/repos/test/project1", host="forge:3000")
        assert repo["name"] == "project1"
        releases = api(
            client,
            "GET",
            "/api/v1/repos/test/project1/releases",
            host="forge:3000",
        )
        tags = sorted(r["tag_name"] for r in releases)
        print(f"Available releases: {tags}")
        assert "v1.0.0" in tags, "v1.0.0 should be available"

    with subtest("Create empty test flake.nix on client"):
        write_empty_flake(client, "/tmp/test-flake/flake.nix")
        client.succeed("cat /tmp/test-flake/flake.nix")

    with subtest("Verify empty flake has no inputs"):
        output = client.succeed("cd /tmp/test-flake && flake-edit list")
        print(f"flake-edit list output (empty): {output}")
        assert "project1" not in output, "project1 should not be in empty flake"

    with subtest("Add input without version pin"):
        output = client.succeed(
            "cd /tmp/test-flake && "
            "flake-edit add project1 git+http://forge:3000/test/project1 --no-flake 2>&1"
        )
        print(f"flake-edit add output: {output}")
        flake_content = client.succeed("cat /tmp/test-flake/flake.nix")
        print(f"flake.nix after add: {flake_content}")
        assert "project1" in flake_content, "project1 should be added"

    with subtest("Pin input to latest version with update --init"):
        output = client.succeed(
            "cd /tmp/test-flake && flake-edit update project1 --init 2>&1"
        )
        print(f"flake-edit update --init output: {output}")
        flake_content = client.succeed("cat /tmp/test-flake/flake.nix")
        print(f"flake.nix after update --init: {flake_content}")
        assert "refs/tags/v1.0.0" in flake_content, (
            "Should be pinned to refs/tags/v1.0.0"
        )

    with subtest("Create additional releases (v1.5.0, v2.0.0)"):
        create_release(forge, "test/project1", "v2.0.0", token=api_token)
        create_release(forge, "test/project1", "v1.5.0", token=api_token)

    with subtest("Test flake-edit update to latest version"):
        output = client.succeed("cd /tmp/test-flake && flake-edit update project1 2>&1")
        print(f"flake-edit update output: {output}")
        updated_flake = client.succeed("cat /tmp/test-flake/flake.nix")
        print(f"Updated flake.nix: {updated_flake}")
        assert "refs/tags/v2.0.0" in updated_flake, "Should be updated to v2.0.0"
        assert "v1.0.0" not in updated_flake, "Should no longer reference v1.0.0"

    with subtest("Verify flake-edit detected Forgejo correctly"):
        print("flake-edit successfully updated from Forgejo instance")

    with subtest("Create 'nixos' organization for channel tests"):
        api(
            forge,
            "POST",
            "/api/v1/orgs",
            token=api_token,
            body={
                "username": "nixos",
                "full_name": "NixOS",
                "visibility": "public",
            },
        )

    with subtest("Create 'nixpkgs' repository under 'nixos' org"):
        api(
            forge,
            "POST",
            "/api/v1/orgs/nixos/repos",
            token=api_token,
            body={
                "auto_init": True,
                "name": "nixpkgs",
                "private": False,
                "default_branch": "nixos-unstable",
            },
        )

    with subtest("Create channel branches for nixpkgs"):
        default_sha = branch_sha(
            forge, "nixos/nixpkgs", "nixos-unstable", token=api_token
        )
        create_branch(
            forge,
            "nixos/nixpkgs",
            "nixos-24.05",
            base_sha=default_sha,
            token=api_token,
        )

    with subtest("Verify nixpkgs branches exist"):
        branches = api(
            client,
            "GET",
            "/api/v1/repos/nixos/nixpkgs/branches",
            host="forge:3000",
        )
        names = sorted(b["name"] for b in branches)
        print(f"Available branches: {names}")
        assert "nixos-24.05" in names, "nixos-24.05 branch should exist"
        assert "nixos-unstable" in names, "nixos-unstable branch should exist"

    with subtest("Create test flake for channel tests"):
        write_empty_flake(client, "/tmp/channel-test/flake.nix")
        client.succeed("cat /tmp/channel-test/flake.nix")

    with subtest("Add nixpkgs input without version pin"):
        output = client.succeed(
            "cd /tmp/channel-test && "
            "flake-edit add nixpkgs git+http://forge:3000/nixos/nixpkgs --no-flake 2>&1"
        )
        print(f"flake-edit add nixpkgs output: {output}")
        flake_content = client.succeed("cat /tmp/channel-test/flake.nix")
        print(f"flake.nix after adding nixpkgs: {flake_content}")
        assert "nixpkgs" in flake_content, "nixpkgs should be added"
        assert "nixos-24" not in flake_content, (
            "Should not have a channel ref yet (unpinned)"
        )

    with subtest("Channel update --init on unpinned input"):
        output = client.succeed(
            "cd /tmp/channel-test && flake-edit update nixpkgs --init 2>&1"
        )
        print(f"flake-edit update --init output: {output}")
        flake_content = client.succeed("cat /tmp/channel-test/flake.nix")
        print(f"flake.nix after update --init: {flake_content}")

    with subtest("Set nixpkgs to nixos-24.05 channel"):
        write_empty_flake(client, "/tmp/channel-test/flake.nix")
        client.succeed(
            "cd /tmp/channel-test && "
            "flake-edit add nixpkgs "
            "'git+http://forge:3000/nixos/nixpkgs?ref=nixos-24.05' --no-flake"
        )
        client.succeed("cat /tmp/channel-test/flake.nix")

    with subtest("Create nixos-24.11 branch"):
        sha = branch_sha(forge, "nixos/nixpkgs", "nixos-24.05", token=api_token)
        create_branch(
            forge, "nixos/nixpkgs", "nixos-24.11", base_sha=sha, token=api_token
        )
        branches = api(
            client,
            "GET",
            "/api/v1/repos/nixos/nixpkgs/branches",
            host="forge:3000",
        )
        names = sorted(b["name"] for b in branches)
        print(f"Branches: {names}")
        assert "nixos-24.11" in names, "nixos-24.11 branch should exist"

    with subtest("Channel update should upgrade from 24.05 to 24.11"):
        output = client.succeed(
            "cd /tmp/channel-test && flake-edit update nixpkgs 2>&1"
        )
        print(f"flake-edit channel update output: {output}")
        flake_content = client.succeed("cat /tmp/channel-test/flake.nix")
        print(f"flake.nix after channel update: {flake_content}")
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
        output = client.succeed(
            "cd /tmp/channel-test && flake-edit update nixpkgs 2>&1"
        )
        print(f"flake-edit update on unstable output: {output}")
        flake_content = client.succeed("cat /tmp/channel-test/flake.nix")
        print(f"flake.nix after update on unstable: {flake_content}")
        assert "nixos-unstable" in flake_content, "Should remain on nixos-unstable"
        assert "nixos-24" not in flake_content, (
            "Should NOT be changed to a stable channel"
        )

    with subtest("Verify nixpkgs- prefix channels also work"):
        sha = branch_sha(forge, "nixos/nixpkgs", "nixos-unstable", token=api_token)
        for new_branch in ("nixpkgs-24.05", "nixpkgs-24.11"):
            create_branch(
                forge,
                "nixos/nixpkgs",
                new_branch,
                base_sha=sha,
                token=api_token,
            )

        write_empty_flake(client, "/tmp/channel-test/flake.nix")
        client.succeed(
            "cd /tmp/channel-test && "
            "flake-edit add nixpkgs "
            "'git+http://forge:3000/nixos/nixpkgs?ref=nixpkgs-24.05' --no-flake"
        )
        output = client.succeed(
            "cd /tmp/channel-test && flake-edit update nixpkgs 2>&1"
        )
        print(f"flake-edit update nixpkgs- prefix output: {output}")
        flake_content = client.succeed("cat /tmp/channel-test/flake.nix")
        print(f"flake.nix after nixpkgs- update: {flake_content}")
        assert "nixpkgs-24.11" in flake_content, "Should be updated to nixpkgs-24.11"
