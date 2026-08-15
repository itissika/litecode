#!/usr/bin/env python3
"""Register Serena as an MCP server in Litecode's global settings DB.

Litecode treats MCP servers separately from Custom Tools:
  - Custom Tool = one stdin-JSON subprocess (single tool)
  - MCP Server  = long-lived stdio JSON-RPC process exposing many tools

Serena is an MCP server, so this is the correct integration path.
Settings UI for MCP is not productized yet; this script writes the DB rows.

Usage:
  python examples/tools/register_serena_mcp.py
  python examples/tools/register_serena_mcp.py --project E:\\litecode
  python examples/tools/register_serena_mcp.py --disable
"""
from __future__ import annotations

import argparse
import json
import os
import shutil
import sqlite3
import sys
from pathlib import Path


SERVER_ID = "serena"
CATALOG_ID = f"mcp_{SERVER_ID}"
POLICY_ALLOW = json.dumps(
    {"default": "allow", "default_id": "__default", "rules": []},
    separators=(",", ":"),
)


def default_db_path() -> Path:
    local = Path(os.environ.get("LOCALAPPDATA", "")) / "litecode" / "litecode.db"
    legacy = Path.home() / ".local" / "share" / "litecode" / "litecode.db"
    if local.is_file():
        return local
    if legacy.is_file():
        return legacy
    return local


def resolve_serena() -> str:
    found = shutil.which("serena")
    if found:
        return found
    candidate = Path.home() / ".local" / "bin" / "serena.exe"
    if candidate.is_file():
        return str(candidate)
    raise SystemExit(
        "serena not found on PATH. Install with:\n"
        "  uv tool install -p 3.13 serena-agent\n"
        "and ensure %USERPROFILE%\\.local\\bin is on PATH."
    )


def build_args(project: str | None, context: str) -> list[str]:
    args = [
        "start-mcp-server",
        f"--context={context}",
        "--open-web-dashboard",
        "false",
    ]
    if project:
        args.extend(["--project", project])
    else:
        args.append("--project-from-cwd")
    return args


def register(conn: sqlite3.Connection, *, project: str | None, context: str) -> None:
    command = resolve_serena()
    args = build_args(project, context)
    transport = {"type": "stdio"}

    conn.execute(
        """
        INSERT INTO mcp_servers (id, command, args_json, env_json, transport_json)
        VALUES (?, ?, ?, ?, ?)
        ON CONFLICT(id) DO UPDATE SET
          command = excluded.command,
          args_json = excluded.args_json,
          env_json = excluded.env_json,
          transport_json = excluded.transport_json
        """,
        (
            SERVER_ID,
            command,
            json.dumps(args),
            json.dumps({}),
            json.dumps(transport),
        ),
    )
    conn.execute(
        """
        INSERT INTO tool_catalog (id, tier, init_scope, catalog_enabled)
        VALUES (?, 'mcp', 'global', 1)
        ON CONFLICT(id) DO UPDATE SET
          tier = excluded.tier,
          init_scope = excluded.init_scope,
          catalog_enabled = 1
        """,
        (CATALOG_ID,),
    )
    # Do not auto-enable on the agent: catalog_enabled only makes the tool
    # bindable. Visibility still requires Settings → Agents → enable mcp_*.
    conn.execute(
        """
        INSERT INTO agent_tools
          (agent_id, tool_id, enabled, policy_json, path_mode, last_applied_preset)
        VALUES ('default', ?, 0, ?, 'unrestricted', NULL)
        ON CONFLICT(agent_id, tool_id) DO UPDATE SET
          policy_json = excluded.policy_json,
          path_mode = excluded.path_mode
        """,
        (CATALOG_ID, POLICY_ALLOW),
    )
    conn.commit()
    print(f"Registered MCP server '{SERVER_ID}'")
    print(f"  command : {command}")
    print(f"  args    : {args}")
    print(f"  catalog : {CATALOG_ID} (catalog_enabled=1)")
    print(f"  agent   : default → {CATALOG_ID} (enabled=0; turn on in Settings → Agents)")
    print()
    print("Restart Litecode serve / desktop so it reloads global settings.")
    print("Then: Tool Catalog (mcp_serena on) → Agents → enable mcp_serena → chat.")


def disable(conn: sqlite3.Connection) -> None:
    conn.execute(
        "UPDATE tool_catalog SET catalog_enabled = 0 WHERE id = ?",
        (CATALOG_ID,),
    )
    conn.execute(
        "UPDATE agent_tools SET enabled = 0 WHERE agent_id = 'default' AND tool_id = ?",
        (CATALOG_ID,),
    )
    conn.commit()
    print(f"Disabled catalog/agent binding for {CATALOG_ID} (mcp_servers row kept).")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--db",
        type=Path,
        default=None,
        help="Path to litecode.db (default: LOCALAPPDATA/litecode/litecode.db)",
    )
    parser.add_argument(
        "--project",
        type=str,
        default=None,
        help="Absolute project path to activate at Serena startup (recommended)",
    )
    parser.add_argument(
        "--context",
        type=str,
        default="ide",
        help="Serena context (ide | oaicompat-agent | desktop-app | ...). Default: ide",
    )
    parser.add_argument(
        "--disable",
        action="store_true",
        help="Disable catalog + agent binding without deleting the server row",
    )
    args = parser.parse_args()

    db = args.db or default_db_path()
    if not db.is_file():
        raise SystemExit(f"litecode.db not found: {db}")

    conn = sqlite3.connect(db)
    try:
        if args.disable:
            disable(conn)
        else:
            project = args.project
            if project:
                project = str(Path(project).resolve())
            register(conn, project=project, context=args.context)
    finally:
        conn.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
