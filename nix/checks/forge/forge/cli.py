"""`forge-cli` console script. Runs inside the test VM, invoked by `forge.client`."""

from __future__ import annotations

import argparse
import json
import logging
import sys
from typing import Sequence

from forge import _logging, api
from forge.api import Auth, ForgeApiError, TokenAuth


def _auth_from_args(args: argparse.Namespace) -> Auth:
    if args.token is not None:
        return TokenAuth(value=args.token)
    return None


def _build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="forge-cli")
    parser.add_argument(
        "--base-url",
        default="http://localhost:3000",
        help="forgejo base URL (default: %(default)s)",
    )
    parser.add_argument("--token", default=None, help="API token")
    parser.add_argument("-v", "--verbose", action="store_true")

    resources = parser.add_subparsers(dest="resource", required=True)

    token = resources.add_parser("token").add_subparsers(dest="verb", required=True)
    t_create = token.add_parser("create")
    t_create.add_argument("--user", required=True)
    t_create.add_argument("--password", required=True)
    t_create.add_argument("--name", required=True)

    repo = resources.add_parser("repo").add_subparsers(dest="verb", required=True)
    r_create = repo.add_parser("create")
    r_create.add_argument("--name", required=True)
    r_create.add_argument("--auto-init", action="store_true")
    r_get = repo.add_parser("get")
    r_get.add_argument("--repo", required=True, help="owner/name")

    release = resources.add_parser("release").add_subparsers(dest="verb", required=True)
    rel_create = release.add_parser("create")
    rel_create.add_argument("--repo", required=True)
    rel_create.add_argument("--tag", required=True)
    rel_list = release.add_parser("list")
    rel_list.add_argument("--repo", required=True)

    org = resources.add_parser("org").add_subparsers(dest="verb", required=True)
    o_create = org.add_parser("create")
    o_create.add_argument("--username", required=True)
    o_create.add_argument("--full-name", required=True)

    org_repo = resources.add_parser("org-repo").add_subparsers(
        dest="verb", required=True
    )
    or_create = org_repo.add_parser("create")
    or_create.add_argument("--org", required=True)
    or_create.add_argument("--name", required=True)
    or_create.add_argument("--auto-init", action="store_true")
    or_create.add_argument("--default-branch", default=None)

    branch = resources.add_parser("branch").add_subparsers(dest="verb", required=True)
    b_create = branch.add_parser("create")
    b_create.add_argument("--repo", required=True)
    b_create.add_argument("--new", required=True)
    b_create.add_argument("--base-sha", required=True)
    b_get = branch.add_parser("get")
    b_get.add_argument("--repo", required=True)
    b_get.add_argument("--branch", required=True)
    b_list = branch.add_parser("list")
    b_list.add_argument("--repo", required=True)

    return parser


def _dispatch(args: argparse.Namespace, auth: Auth) -> object:
    base = args.base_url
    match (args.resource, args.verb):
        case ("token", "create"):
            return api.create_token(
                base, user=args.user, password=args.password, name=args.name
            )
        case ("repo", "create"):
            return api.create_repo(
                base, auth=auth, name=args.name, auto_init=args.auto_init
            )
        case ("repo", "get"):
            return api.get_repo(base, auth=auth, repo=args.repo)
        case ("release", "create"):
            return api.create_release(base, auth=auth, repo=args.repo, tag=args.tag)
        case ("release", "list"):
            return api.list_releases(base, auth=auth, repo=args.repo)
        case ("org", "create"):
            return api.create_org(
                base, auth=auth, username=args.username, full_name=args.full_name
            )
        case ("org-repo", "create"):
            return api.create_org_repo(
                base,
                auth=auth,
                org=args.org,
                name=args.name,
                auto_init=args.auto_init,
                default_branch=args.default_branch,
            )
        case ("branch", "create"):
            return api.create_branch(
                base,
                auth=auth,
                repo=args.repo,
                new=args.new,
                base_sha=args.base_sha,
            )
        case ("branch", "get"):
            return api.get_branch(base, auth=auth, repo=args.repo, branch=args.branch)
        case ("branch", "list"):
            return api.list_branches(base, auth=auth, repo=args.repo)
        case _:
            raise SystemExit(f"unknown command: {args.resource} {args.verb}")


def main(argv: Sequence[str] | None = None) -> int:
    parser = _build_parser()
    args = parser.parse_args(argv)
    _logging.configure(level=logging.DEBUG if args.verbose else logging.WARNING)
    auth = _auth_from_args(args)
    try:
        result = _dispatch(args, auth)
    except ForgeApiError as exc:
        json.dump(
            {
                "error": str(exc),
                "status": exc.status,
                "method": exc.method,
                "url": exc.url,
                "body": exc.body,
            },
            sys.stderr,
        )
        sys.stderr.write("\n")
        return 1
    json.dump(result, sys.stdout)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
