#!/usr/bin/env bash
set -euo pipefail
exec "$(dirname "$0")/flock_hook.sh" inject_session_end
