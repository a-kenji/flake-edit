"""Forgejo HTTP primitives. Runs inside the test VM, invoked by `forge.cli`."""

from __future__ import annotations

import base64
import json
import logging
import urllib.error
import urllib.request
from dataclasses import dataclass
from typing import Any

from forge.schemas import Branch, Release, Repo, Token

logger = logging.getLogger("forge.api")

JsonValue = Any


@dataclass(frozen=True)
class TokenAuth:
    value: str


@dataclass(frozen=True)
class BasicAuth:
    user: str
    password: str


Auth = TokenAuth | BasicAuth | None


class ForgeApiError(RuntimeError):
    def __init__(self, *, status: int, method: str, url: str, body: str) -> None:
        super().__init__(f"{method} {url} -> HTTP {status}: {body}")
        self.status = status
        self.method = method
        self.url = url
        self.body = body


def _auth_headers(auth: Auth) -> dict[str, str]:
    if auth is None:
        return {}
    if isinstance(auth, TokenAuth):
        return {"Authorization": f"token {auth.value}"}
    creds = f"{auth.user}:{auth.password}".encode()
    return {"Authorization": "Basic " + base64.b64encode(creds).decode()}


def request(
    method: str,
    base_url: str,
    path: str,
    *,
    auth: Auth = None,
    body: JsonValue = None,
) -> JsonValue:
    url = base_url.rstrip("/") + path
    headers = {"Accept": "application/json", **_auth_headers(auth)}
    data: bytes | None = None
    if body is not None:
        headers["Content-Type"] = "application/json"
        data = json.dumps(body).encode("utf-8")
    req = urllib.request.Request(url, data=data, method=method, headers=headers)
    logger.debug("%s %s", method, url)
    try:
        with urllib.request.urlopen(req) as resp:
            raw = resp.read().decode("utf-8")
    except urllib.error.HTTPError as exc:
        err_body = exc.read().decode("utf-8", errors="replace")
        raise ForgeApiError(
            status=exc.code, method=method, url=url, body=err_body
        ) from exc
    if not raw.strip():
        return None
    return json.loads(raw)


def create_token(base_url: str, *, user: str, password: str, name: str) -> Token:
    resp = request(
        "POST",
        base_url,
        f"/api/v1/users/{user}/tokens",
        auth=BasicAuth(user=user, password=password),
        body={"name": name, "scopes": ["all"]},
    )
    return resp


def create_repo(
    base_url: str, *, auth: Auth, name: str, auto_init: bool = True
) -> Repo:
    return request(
        "POST",
        base_url,
        "/api/v1/user/repos",
        auth=auth,
        body={"auto_init": auto_init, "name": name, "private": False},
    )


def get_repo(base_url: str, *, auth: Auth, repo: str) -> Repo:
    return request("GET", base_url, f"/api/v1/repos/{repo}", auth=auth)


def create_release(base_url: str, *, auth: Auth, repo: str, tag: str) -> Release:
    return request(
        "POST",
        base_url,
        f"/api/v1/repos/{repo}/releases",
        auth=auth,
        body={
            "tag_name": tag,
            "name": f"Release {tag}",
            "body": f"Test release {tag}",
        },
    )


def list_releases(base_url: str, *, auth: Auth, repo: str) -> list[Release]:
    return request("GET", base_url, f"/api/v1/repos/{repo}/releases", auth=auth)


def create_org(
    base_url: str, *, auth: Auth, username: str, full_name: str
) -> JsonValue:
    return request(
        "POST",
        base_url,
        "/api/v1/orgs",
        auth=auth,
        body={
            "username": username,
            "full_name": full_name,
            "visibility": "public",
        },
    )


def create_org_repo(
    base_url: str,
    *,
    auth: Auth,
    org: str,
    name: str,
    auto_init: bool = True,
    default_branch: str | None = None,
) -> Repo:
    body: dict[str, Any] = {
        "auto_init": auto_init,
        "name": name,
        "private": False,
    }
    if default_branch is not None:
        body["default_branch"] = default_branch
    return request("POST", base_url, f"/api/v1/orgs/{org}/repos", auth=auth, body=body)


def get_branch(base_url: str, *, auth: Auth, repo: str, branch: str) -> Branch:
    return request(
        "GET", base_url, f"/api/v1/repos/{repo}/branches/{branch}", auth=auth
    )


def create_branch(
    base_url: str, *, auth: Auth, repo: str, new: str, base_sha: str
) -> Branch:
    return request(
        "POST",
        base_url,
        f"/api/v1/repos/{repo}/branches",
        auth=auth,
        body={"new_branch_name": new, "old_ref_name": base_sha},
    )


def list_branches(base_url: str, *, auth: Auth, repo: str) -> list[Branch]:
    return request("GET", base_url, f"/api/v1/repos/{repo}/branches", auth=auth)
