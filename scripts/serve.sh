#!/usr/bin/env bash
# Start litecode with binaries built from the current tree (never a stale target/release/litecode).
#
# Usage:
#   ./scripts/serve.sh                 # API (host cargo) + Vite dev UI
#   ./scripts/serve.sh --api-only      # API only
#   ./scripts/serve.sh --web-only      # Vite dev UI only (API must already be running)
#   ./scripts/serve.sh --release       # cargo run --release (rebuilds release first)
#   ./scripts/serve.sh --no-cleanup    # skip killing stale processes / freeing ports
#   ./scripts/serve.sh -- --workspace /path/to/project
#
# One process = one workspace. Changing folder requires restarting this script
# (no in-process hot switch). Electron end-state: ./scripts/dev_win.ps1 (Windows)
# or the desktop package.
#
# Environment:
#   LITECODE_BIND      default 127.0.0.1:7483
#   LITECODE_AGENT     default default
#   LITECODE_WORKSPACE optional; passed as --workspace when set
#
# Default is always host cargo.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

WEB_DIR="$PROJECT_DIR/web"

BIND="${LITECODE_BIND:-127.0.0.1:7483}"
AGENT="${LITECODE_AGENT:-default}"
WEB_PORT="${LITECODE_WEB_PORT:-5173}"
API_ONLY=0
WEB_ONLY=0
USE_RELEASE=0
SKIP_CLEANUP=0
EXTRA_ARGS=()

usage() {
    # Print the full leading comment block (after the shebang), stripping the
    # '# ' prefix, stopping at the first non-comment line. Never truncates.
    awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$0"
    exit "${1:-0}"
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        -h|--help)
            usage 0
            ;;
        --api-only)
            API_ONLY=1
            shift
            ;;
        --web-only)
            WEB_ONLY=1
            shift
            ;;
        --release)
            USE_RELEASE=1
            shift
            ;;
        --no-cleanup)
            SKIP_CLEANUP=1
            shift
            ;;
        --bind)
            BIND="${2:?missing value for --bind}"
            shift 2
            ;;
        --agent)
            AGENT="${2:?missing value for --agent}"
            shift 2
            ;;
        --)
            shift
            EXTRA_ARGS+=("$@")
            break
            ;;
        *)
            EXTRA_ARGS+=("$1")
            shift
            ;;
    esac
done

if [[ "$API_ONLY" -eq 1 && "$WEB_ONLY" -eq 1 ]]; then
    echo "error: --api-only and --web-only are mutually exclusive" >&2
    exit 1
fi

cd "$PROJECT_DIR"

if [[ "$BIND" == *:* ]]; then
    API_PORT="${BIND##*:}"
    API_HOST="${BIND%:*}"
else
    API_PORT="$BIND"
    API_HOST="127.0.0.1"
fi

# Return listening PIDs on tcp:$port (one per line), or nothing.
pids_on_port() {
    local port="$1"
    if command -v lsof >/dev/null 2>&1; then
        lsof -ti "tcp:$port" -sTCP:LISTEN 2>/dev/null || true
    elif command -v fuser >/dev/null 2>&1; then
        fuser "$port/tcp" 2>/dev/null | tr ' ' '\n' | grep -E '^[0-9]+$' || true
    fi
}

# Send TERM, brief wait, then KILL any survivors.
stop_pids() {
    local pid reason="$1"
    shift
    local -a victims=("$@")
    local still_alive=()

    for pid in "${victims[@]}"; do
        [[ -z "$pid" ]] && continue
        if kill -0 "$pid" 2>/dev/null; then
            echo "    stopping pid $pid ($reason)"
            kill -TERM "$pid" 2>/dev/null || true
            still_alive+=("$pid")
        fi
    done

    if [[ ${#still_alive[@]} -eq 0 ]]; then
        return 0
    fi

    sleep 0.5

    for pid in "${still_alive[@]}"; do
        if kill -0 "$pid" 2>/dev/null; then
            echo "    force-killing pid $pid ($reason)"
            kill -KILL "$pid" 2>/dev/null || true
        fi
    done
}

collect_unique_pids() {
    sort -u | grep -E '^[0-9]+$' || true
}

free_port() {
    local port="$1"
    local label="$2"
    local -a pids=()

    while IFS= read -r pid; do
        pids+=("$pid")
    done < <(pids_on_port "$port" | collect_unique_pids)

    if [[ ${#pids[@]} -eq 0 ]]; then
        return 0
    fi

    echo "==> freeing $label (port $port)"
    stop_pids "$label" "${pids[@]}"
}

# Stop prior litecode serve binaries built from this tree (debug or release).
kill_stale_litecode() {
    local -a binaries=(
        "${PROJECT_DIR}/target/debug/litecode"
        "${PROJECT_DIR}/target/release/litecode"
    )
    local binary pid
    local -a pids=()

    # Match only litecode serve binaries from this tree that are actually
    # serving the port we are about to use, so we never kill an unrelated
    # instance bound to another port. Skip our own shell and its parent.
    for binary in "${binaries[@]}"; do
        while IFS= read -r pid; do
            [[ "$pid" == "$$" || "$pid" == "$PPID" ]] && continue
            pids+=("$pid")
        done < <(pgrep -f "${binary}.*serve.*:${API_PORT}" 2>/dev/null || true)
    done

    if [[ ${#pids[@]} -eq 0 ]]; then
        return 0
    fi

    echo "==> stopping stale litecode serve from this tree (port $API_PORT)"
    # shellcheck disable=SC2046
    stop_pids "stale litecode serve" $(printf '%s\n' "${pids[@]}" | collect_unique_pids)
}

# Stop Vite dev server previously started for this web/ directory.
kill_stale_vite() {
    local -a pids=()

    while IFS= read -r pid; do
        pids+=("$pid")
    done < <(pgrep -f "node.*${WEB_DIR}.*vite" 2>/dev/null || true)

    if [[ ${#pids[@]} -eq 0 ]]; then
        return 0
    fi

    echo "==> stopping stale Vite dev server for web/"
    # shellcheck disable=SC2046
    stop_pids "stale Vite dev server" $(printf '%s\n' "${pids[@]}" | collect_unique_pids)
}

cleanup_before_start() {
    if [[ "$SKIP_CLEANUP" -eq 1 ]]; then
        echo "==> skipping pre-start cleanup (--no-cleanup)"
        return 0
    fi

    echo "==> cleaning up stale processes and ports..."

    if [[ "$WEB_ONLY" -eq 1 ]]; then
        kill_stale_vite
        free_port "$WEB_PORT" "Vite dev UI"
    elif [[ "$API_ONLY" -eq 1 ]]; then
        kill_stale_litecode
        free_port "$API_PORT" "litecode API"
    else
        kill_stale_litecode
        kill_stale_vite
        free_port "$API_PORT" "litecode API"
        free_port "$WEB_PORT" "Vite dev UI"
    fi
}

PIDS=()
cleanup() {
    local pid
    for pid in "${PIDS[@]}"; do
        if kill -0 "$pid" 2>/dev/null; then
            kill "$pid" 2>/dev/null || true
        fi
    done
    wait 2>/dev/null || true
}
trap cleanup EXIT INT TERM

run_api_local() {
    local -a cargo_args=(run)
    if [[ "$USE_RELEASE" -eq 1 ]]; then
        echo "==> building release binary from current sources..."
        cargo build --release
        cargo_args+=(--release)
    else
        echo "==> starting API via cargo run (rebuilds when sources change)..."
    fi

    cargo_args+=(--)
    if [[ -n "${LITECODE_WORKSPACE:-}" ]]; then
        cargo_args+=(--workspace "$LITECODE_WORKSPACE")
        echo "    workspace=$LITECODE_WORKSPACE (process will chdir here)"
    fi
    cargo_args+=(serve --bind "$BIND" --agent "$AGENT")
    if [[ ${#EXTRA_ARGS[@]} -gt 0 ]]; then
        cargo_args+=("${EXTRA_ARGS[@]}")
    fi

    echo "    bind=$BIND agent=$AGENT [host]"
    echo "    health: http://$BIND/health"
    echo "    ws:     ws://$BIND/ws"
    LITECODE_CHANNEL=dev cargo "${cargo_args[@]}" &
    PIDS+=("$!")
}

run_web() {
    if [[ ! -d "$WEB_DIR" ]]; then
        echo "error: web directory not found: $WEB_DIR" >&2
        exit 1
    fi
    if [[ ! -d "$WEB_DIR/node_modules" ]]; then
        echo "==> installing web dependencies..."
        (cd "$WEB_DIR" && npm install)
    fi
    echo "==> starting Vite dev server (proxies /api and /ws to $BIND)..."
    echo "    UI: http://127.0.0.1:$WEB_PORT"
    (cd "$WEB_DIR" && npm run dev) &
    PIDS+=("$!")
}

cleanup_before_start

if [[ "$WEB_ONLY" -eq 1 ]]; then
    run_web
elif [[ "$API_ONLY" -eq 1 ]]; then
    run_api_local
else
    run_api_local
    # Wait until the API is actually listening before starting Vite.
    # Rust compilation can take several minutes — wait up to 10 minutes.
    echo "==> waiting for API to be ready at http://$BIND/health ..."
    for i in $(seq 1 600); do
        if curl -sf "http://$BIND/health" >/dev/null 2>&1; then
            echo ""
            echo "==> API is ready"
            break
        fi
        if [[ $((i % 10)) -eq 0 ]]; then
            printf "."
        fi
        sleep 1
    done
    run_web
fi

echo ""
echo "Press Ctrl+C to stop."
wait -n
exit $?
