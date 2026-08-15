#!/usr/bin/env python3
"""Demo custom tool: echoes input and reports backend execution context.

Use this to verify the custom-tool pipeline end to end.

Protocol (see src/tools/custom.rs):
  - reads a JSON object from stdin (schema properties are at the top level)
  - writes the result to stdout
  - exit 0 = success, exit 2 = blocked, any other code = error
"""
from __future__ import annotations

import datetime
import json
import os
import sys


def main() -> int:
    raw = sys.stdin.read()
    try:
        data = json.loads(raw) if raw.strip() else {}
    except json.JSONDecodeError as exc:
        print(f"invalid json input: {exc}", file=sys.stderr)
        return 1

    if not isinstance(data, dict):
        print("input must be a JSON object", file=sys.stderr)
        return 1

    text = data.get("text")
    if not isinstance(text, str) or not text:
        print("missing required string field: text", file=sys.stderr)
        return 1

    shout = bool(data.get("shout", False))
    echoed = text.upper() if shout else text

    result = {
        "echo": echoed,
        "execution_time": datetime.datetime.now().isoformat(timespec="seconds"),
        "cwd": os.getcwd(),
        "note": "This result came from the demo custom tool subprocess.",
    }
    print(json.dumps(result, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
