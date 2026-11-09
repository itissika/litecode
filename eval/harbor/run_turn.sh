#!/usr/bin/env bash
# One LiteCode turn inside the task container. Uploaded by litecode_agent.py.
#
# The instruction is argv[1]: it is multi-line and argv carries it without any
# quoting games. Output is teed so the artifact keeps what the CLI printed,
# including the usage line this run reports.
set -o pipefail

litecode --workspace "$PWD" --yes "$1" </dev/null 2>&1 | tee /logs/agent/litecode.txt
status=${PIPESTATUS[0]}

# Session trace: the JSON log the CLI writes plus the session database the UI
# would read. Both sit under the workspace LiteCode was given.
mkdir -p /logs/agent/litecode
cp -r "$PWD/.litecode/logs" /logs/agent/litecode/ 2>/dev/null || true
cp "$PWD"/.litecode/sessions.db* /logs/agent/litecode/ 2>/dev/null || true

exit "$status"
