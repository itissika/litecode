#!/usr/bin/env bash
set -euo pipefail
exec "$(dirname "$0")/flock_hook.sh" inject_post_tool_results_by_file
