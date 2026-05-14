"""Typed helpers for the NixOS VM integration test.

Installed in two places: as a Python library on the host (used by the test
driver) and as a system package inside the VM (which exposes the `forge-cli`
console script).
"""

from __future__ import annotations

from forge.client import ForgeClient
from forge.runner import run

__all__ = ["ForgeClient", "run"]
