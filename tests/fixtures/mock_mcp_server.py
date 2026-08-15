#!/usr/bin/env python3
"""Minimal mock MCP server (line-delimited JSON-RPC) for litecode MCP tests.

Speaks the subset of the MCP stdio transport that litecode's
`McpStdioClient` uses: newline-delimited JSON-RPC 2.0 on stdin/stdout.

Supported tools:
  - "echo": returns {"text": <arguments>}
  - "crash": exits the process immediately (exercises pool crash-rebuild).
  - "hang": sleeps forever (exercises the 60s timeout kill path).
"""

import json
import os
import sys
import time

INITIALIZED = {"initialized": False}


def send(obj):
    sys.stdout.write(json.dumps(obj, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def handle(method, params, req_id):
    if method == "initialize":
        INITIALIZED["initialized"] = True
        return {
            "protocolVersion": "2024-11-05",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "mock-mcp", "version": "0.0.0"},
        }
    if method == "notifications/initialized":
        # No response for a notification.
        return None
    if method == "tools/list":
        return {
            "tools": [
                {
                    "name": "echo",
                    "description": "Echo the arguments back",
                    "inputSchema": {"type": "object", "properties": {}},
                },
                {
                    "name": "crash",
                    "description": "Crash the server process",
                    "inputSchema": {"type": "object", "properties": {}},
                },
                {
                    "name": "hang",
                    "description": "Hang until killed",
                    "inputSchema": {"type": "object", "properties": {}},
                },
            ]
        }
    if method == "tools/call":
        name = params.get("name")
        arguments = params.get("arguments", {})
        if name == "echo":
            return {
                "content": [
                    {"type": "text", "text": json.dumps(arguments, separators=(",", ":"))}
                ],
                "isError": False,
            }
        if name == "crash":
            os._exit(0)
        if name == "hang":
            # Never returns; the test kills the process on timeout.
            time.sleep(3600)
            return {"content": [], "isError": False}
        return {
            "content": [{"type": "text", "text": f"unknown tool {name}"}],
            "isError": True,
        }
    return {"_unknownMethod": method}


def main():
    for line in sys.stdin:
        if not line:
            break
        line = line.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            send({"jsonrpc": "2.0", "id": None, "error": {"code": -32700, "message": "parse error"}})
            continue
        req_id = msg.get("id")
        if req_id is None:
            # Notification — no response.
            continue
        method = msg.get("method", "")
        params = msg.get("params") or {}
        try:
            result = handle(method, params, req_id)
        except Exception as exc:  # noqa: BLE001
            send({"jsonrpc": "2.0", "id": req_id, "error": {"code": -32603, "message": str(exc)}})
            continue
        if result is None:
            continue
        send({"jsonrpc": "2.0", "id": req_id, "result": result})


if __name__ == "__main__":
    main()
