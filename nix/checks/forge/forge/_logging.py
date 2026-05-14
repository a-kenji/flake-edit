"""
Logs go to stderr so `forge-cli`'s stdout stays a clean JSON channel for
the host-side `ForgeClient` to parse.
"""

from __future__ import annotations

import logging
import sys

_FORMAT = "%(asctime)s %(levelname)-7s %(name)s: %(message)s"


def configure(level: int = logging.INFO) -> None:
    root = logging.getLogger()
    # Guard so repeat calls (runner + cli both call configure) don't stack handlers.
    if not root.handlers:
        handler = logging.StreamHandler(sys.stderr)
        handler.setFormatter(logging.Formatter(_FORMAT))
        root.addHandler(handler)
    root.setLevel(level)
