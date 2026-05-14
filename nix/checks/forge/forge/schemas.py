"""Typed shapes for forgejo API responses. Used on both sides of the host/guest divide."""

from __future__ import annotations

from typing import TypedDict


class Token(TypedDict):
    sha1: str


class Repo(TypedDict):
    name: str


class Release(TypedDict):
    tag_name: str


class BranchCommit(TypedDict):
    id: str


class Branch(TypedDict):
    name: str
    commit: BranchCommit
