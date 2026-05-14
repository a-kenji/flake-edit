"""Typed host-side facade over `forge-cli`. Runs on the host (inside `nixos-test-driver`)."""

from __future__ import annotations

import json
import logging
import shlex
from typing import TYPE_CHECKING, Any

from forge.schemas import Branch, Release, Repo, Token

if TYPE_CHECKING:
    from test_driver.machine import QemuMachine

logger = logging.getLogger("forge.client")


class ForgeClient:
    def __init__(
        self,
        machine: QemuMachine,
        *,
        base_url: str = "http://localhost:3000",
        token: str | None = None,
    ) -> None:
        self.machine = machine
        self.base_url = base_url
        self.token: str | None = token

    def set_token(self, token: str) -> None:
        self.token = token

    def _run(self, *args: str) -> Any:
        cmd: list[str] = ["forge-cli", "--base-url", self.base_url]
        if self.token is not None:
            cmd += ["--token", self.token]
        cmd += list(args)
        joined = shlex.join(cmd)
        logger.debug("succeed: %s", joined)
        raw = self.machine.succeed(joined)
        return json.loads(raw) if raw.strip() else None

    def create_token(self, *, user: str, password: str, name: str) -> str:
        result: Token = self._run(
            "token",
            "create",
            "--user",
            user,
            "--password",
            password,
            "--name",
            name,
        )
        return result["sha1"]

    def create_repo(self, *, name: str, auto_init: bool = True) -> Repo:
        args = ["repo", "create", "--name", name]
        if auto_init:
            args.append("--auto-init")
        return self._run(*args)

    def get_repo(self, *, repo: str) -> Repo:
        return self._run("repo", "get", "--repo", repo)

    def create_release(self, *, repo: str, tag: str) -> Release:
        return self._run("release", "create", "--repo", repo, "--tag", tag)

    def list_releases(self, *, repo: str) -> list[Release]:
        return self._run("release", "list", "--repo", repo)

    def create_org(self, *, username: str, full_name: str) -> Any:
        return self._run(
            "org", "create", "--username", username, "--full-name", full_name
        )

    def create_org_repo(
        self,
        *,
        org: str,
        name: str,
        auto_init: bool = True,
        default_branch: str | None = None,
    ) -> Repo:
        args = ["org-repo", "create", "--org", org, "--name", name]
        if auto_init:
            args.append("--auto-init")
        if default_branch is not None:
            args += ["--default-branch", default_branch]
        return self._run(*args)

    def create_branch(self, *, repo: str, new: str, base_sha: str) -> Branch:
        return self._run(
            "branch",
            "create",
            "--repo",
            repo,
            "--new",
            new,
            "--base-sha",
            base_sha,
        )

    def get_branch(self, *, repo: str, branch: str) -> Branch:
        return self._run("branch", "get", "--repo", repo, "--branch", branch)

    def branch_sha(self, *, repo: str, branch: str) -> str:
        info = self.get_branch(repo=repo, branch=branch)
        return info["commit"]["id"]

    def list_branches(self, *, repo: str) -> list[Branch]:
        return self._run("branch", "list", "--repo", repo)
