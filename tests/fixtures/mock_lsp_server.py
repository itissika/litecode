#!/usr/bin/env python3
"""Minimal mock language server for litecode LSP integration tests."""

import json
import os
import sys
import time

# Test hook: when MOCK_LSP_PID_FILE is set, write our pid so a test can assert
# the process is reaped after the hub is dropped.
_pid_file = os.environ.get("MOCK_LSP_PID_FILE")
if _pid_file:
    with open(_pid_file, "w") as _f:
        _f.write(str(os.getpid()))


def read_message():
    headers = {}
    while True:
        line = sys.stdin.readline()
        if not line:
            return None
        line = line.strip()
        if not line:
            break
        key, _, value = line.partition(":")
        headers[key.strip().lower()] = value.strip()
    length = int(headers.get("content-length", 0))
    if length == 0:
        return None
    body = sys.stdin.read(length)
    return json.loads(body)


def write_message(obj):
    body = json.dumps(obj, separators=(",", ":"))
    sys.stdout.write(f"Content-Length: {len(body)}\r\n\r\n{body}")
    sys.stdout.flush()


def publish_diagnostics(uri):
    """MOCK_LSP_DIAG=error|none|delay_error — default none (no notification)."""
    mode = os.environ.get("MOCK_LSP_DIAG", "none").strip().lower()
    if mode in ("", "none", "0", "off"):
        return
    delay_ms = int(os.environ.get("MOCK_LSP_DIAG_DELAY_MS", "0") or "0")
    if delay_ms > 0:
        time.sleep(delay_ms / 1000.0)
    if mode in ("delay_error", "error", "1", "on"):
        diags = [
            {
                "severity": 1,
                "message": "mock error",
                "range": {
                    "start": {"line": 0, "character": 0},
                    "end": {"line": 0, "character": 1},
                },
            }
        ]
    elif mode == "warn_only":
        diags = [
            {
                "severity": 2,
                "message": "mock warn",
                "range": {
                    "start": {"line": 0, "character": 0},
                    "end": {"line": 0, "character": 1},
                },
            }
        ]
    else:
        return
    write_message(
        {
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {"uri": uri, "diagnostics": diags},
        }
    )


last_version = {}
pending_hover = []


def main():
    while True:
        msg = read_message()
        if msg is None:
            break
        method = msg.get("method", "")
        # Notifications (didOpen / didChange) — optional diagnostics push.
        if "id" not in msg:
            if method in ("textDocument/didOpen", "textDocument/didChange"):
                td = msg.get("params", {}).get("textDocument", {})
                uri = td.get("uri")
                ver = td.get("version")
                if uri is not None and ver is not None:
                    last_version[uri] = ver
                if uri:
                    publish_diagnostics(uri)
            continue
        req_id = msg["id"]
        if method == "initialize":
            write_message(
                {
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": {"capabilities": {"textDocumentSync": 2}},
                }
            )
        elif os.environ.get("MOCK_LSP_HANG") == "1" and method == "textDocument/hover":
            continue
        elif method == "textDocument/definition":
            pos = msg["params"]["position"]
            line = int(pos.get("line", 0))
            ch = int(pos.get("character", 0))
            write_message(
                {
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": {
                        "uri": msg["params"]["textDocument"]["uri"],
                        "range": {
                            "start": {"line": line, "character": ch},
                            "end": {"line": line, "character": ch + 3},
                        },
                    },
                }
            )
        elif method == "textDocument/hover":
            ch = int(msg["params"]["position"].get("character", 0))
            payload = {
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {
                    "contents": {
                        "kind": "markdown",
                        "value": f"mock hover @c={ch}",
                    }
                },
            }
            if os.environ.get("MOCK_LSP_REVERSE_HOVER") == "1":
                pending_hover.append(payload)
                if len(pending_hover) >= 2:
                    for item in reversed(pending_hover):
                        write_message(item)
                    pending_hover.clear()
                continue
            write_message(payload)
        elif method == "textDocument/completion":
            uri = msg["params"]["textDocument"]["uri"]
            write_message(
                {
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": {
                        "isIncomplete": False,
                        "items": [],
                        "litecodeMockVersion": last_version.get(uri),
                    },
                }
            )
        elif method == "textDocument/references":
            uri = msg["params"]["textDocument"]["uri"]
            ch = int(msg["params"]["position"].get("character", 0))
            write_message(
                {
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": [
                        {
                            "uri": uri,
                            "range": {
                                "start": {"line": 0, "character": 0},
                                "end": {"line": 0, "character": 3},
                            },
                        },
                        {
                            "uri": uri,
                            "range": {
                                "start": {"line": 1, "character": max(ch, 0)},
                                "end": {"line": 1, "character": max(ch, 0) + 3},
                            },
                        },
                    ],
                }
            )
        elif method == "textDocument/implementation":
            write_message(
                {
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": {
                        "uri": msg["params"]["textDocument"]["uri"],
                        "range": {
                            "start": {"line": 1, "character": 0},
                            "end": {"line": 1, "character": 4},
                        },
                    },
                }
            )
        elif method == "textDocument/documentSymbol":
            write_message(
                {
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": [
                        {
                            "name": "mock_fn",
                            "kind": 12,
                            "range": {
                                "start": {"line": 0, "character": 0},
                                "end": {"line": 2, "character": 0},
                            },
                            "selectionRange": {
                                "start": {"line": 0, "character": 0},
                                "end": {"line": 0, "character": 7},
                            },
                        }
                    ],
                }
            )
        elif method == "workspace/symbol":
            write_message(
                {
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": [
                        {
                            "name": "MockSymbol",
                            "kind": 5,
                            "location": {
                                "uri": f"file://{os.getcwd()}/lib.rs",
                                "range": {
                                    "start": {"line": 0, "character": 0},
                                    "end": {"line": 0, "character": 10},
                                },
                            },
                        }
                    ],
                }
            )
        elif method == "textDocument/prepareCallHierarchy":
            uri = msg["params"]["textDocument"]["uri"]
            write_message(
                {
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": [
                        {
                            "name": "mock_caller",
                            "kind": 12,
                            "uri": uri,
                            "range": {
                                "start": {"line": 0, "character": 0},
                                "end": {"line": 1, "character": 0},
                            },
                            "selectionRange": {
                                "start": {"line": 0, "character": 0},
                                "end": {"line": 0, "character": 4},
                            },
                        }
                    ],
                }
            )
        elif method in ("callHierarchy/incomingCalls", "callHierarchy/outgoingCalls"):
            item = msg["params"]["item"]
            write_message(
                {
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": [
                        {
                            "from" if "incoming" in method else "to": item,
                            "fromRanges": [
                                {
                                    "start": {"line": 0, "character": 0},
                                    "end": {"line": 0, "character": 4},
                                }
                            ],
                        }
                    ],
                }
            )
        else:
            write_message({"jsonrpc": "2.0", "id": req_id, "result": None})


if __name__ == "__main__":
    main()
