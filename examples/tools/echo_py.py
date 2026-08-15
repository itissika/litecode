#!/usr/bin/env python3
"""Custom tool: echo a message from stdin JSON."""
from __future__ import annotations

import json
import sys


def main() -> int:
    try:
        data = json.load(sys.stdin)
    except json.JSONDecodeError as exc:
        print(f"invalid json input: {exc}", file=sys.stderr)
        return 1

    message = data.get("message")
    if not isinstance(message, str) or not message:
        print("missing required string field: message", file=sys.stderr)
        return 1

    print(message)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
