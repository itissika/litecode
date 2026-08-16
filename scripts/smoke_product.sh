#!/usr/bin/env bash
# Smoke-test a product tree from a non-repo cwd.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PRODUCT="${1:-$ROOT/dist/product}"
TOKEN="smoke-token-$(date +%s)"
WS_A="$(mktemp -d /tmp/litecode-ws-a-XXXXXX)"
WS_B="$(mktemp -d /tmp/litecode-ws-b-XXXXXX)"
LOG="$(mktemp /tmp/litecode-smoke-XXXXXX.log)"

cleanup() {
  if [[ -n "${PID1:-}" ]]; then kill "$PID1" 2>/dev/null || true; fi
  if [[ -n "${PID2:-}" ]]; then kill "$PID2" 2>/dev/null || true; fi
  if [[ -n "${PID_R:-}" ]]; then kill "$PID_R" 2>/dev/null || true; fi
  rm -rf "$WS_A" "$WS_B"
  rm -f "$LOG" "${LOG_R:-}"
}
trap cleanup EXIT

if [[ ! -x "$PRODUCT/litecode" && ! -x "$PRODUCT/litecode.exe" ]]; then
  echo "missing litecode binary in $PRODUCT — run scripts/assemble_product.sh first" >&2
  exit 1
fi
BIN="$PRODUCT/litecode"
[[ -x "$PRODUCT/litecode.exe" ]] && BIN="$PRODUCT/litecode.exe"

echo "==> smoke from cwd=/tmp using $BIN"
cd /tmp

export LITECODE_TOKEN="$TOKEN"
"$BIN" serve --bind 127.0.0.1:0 --require-auth --workspace "$WS_A" >"$LOG" 2>&1 &
PID1=$!

READY=""
for _ in $(seq 1 100); do
  if READY="$(grep -m1 '^LITECODE_READY ' "$LOG" || true)"; then
    [[ -n "$READY" ]] && break
  fi
  sleep 0.1
done
if [[ -z "$READY" ]]; then
  echo "timeout waiting for LITECODE_READY" >&2
  cat "$LOG" >&2 || true
  exit 1
fi
BASE="${READY#LITECODE_READY }"
BASE="${BASE%/}"
echo "    $READY"

curl -fsS "$BASE/health" | grep -q '"ok":true'
curl -fsS -o /dev/null -w "%{http_code}" -H "Authorization: Bearer $TOKEN" "$BASE/api/settings" | grep -q 200
CODE="$(curl -s -o /dev/null -w "%{http_code}" "$BASE/api/settings" || true)"
[[ "$CODE" == "401" ]] || { echo "expected 401 without token, got $CODE" >&2; exit 1; }

# Second process, different workspace — should succeed
LOG2="$(mktemp /tmp/litecode-smoke2-XXXXXX.log)"
"$BIN" serve --bind 127.0.0.1:0 --require-auth --workspace "$WS_B" >"$LOG2" 2>&1 &
PID2=$!
READY2=""
for _ in $(seq 1 100); do
  READY2="$(grep -m1 '^LITECODE_READY ' "$LOG2" || true)"
  [[ -n "$READY2" ]] && break
  sleep 0.1
done
[[ -n "$READY2" ]] || { echo "second instance failed to start" >&2; cat "$LOG2" >&2; exit 1; }

# Third attempt on WS_A while PID1 holds it — should fail to start
set +e
"$BIN" serve --bind 127.0.0.1:0 --require-auth --workspace "$WS_A" >"$LOG.conflict" 2>&1
RC=$?
set -e
[[ "$RC" -ne 0 ]] || { echo "expected lock conflict on same workspace" >&2; cat "$LOG.conflict" >&2; exit 1; }
grep -qiE 'already open|lock busy' "$LOG.conflict" || {
  echo "conflict output missing lock message:" >&2
  cat "$LOG.conflict" >&2
  exit 1
}

# Process restart: stop A, then start A again (lock released on exit).
echo "==> restart workspace A after stop"
kill "$PID1" 2>/dev/null || true
wait "$PID1" 2>/dev/null || true
PID1=""
LOG_R="$(mktemp /tmp/litecode-smoke-restart-XXXXXX.log)"
"$BIN" --workspace "$WS_A" serve --bind 127.0.0.1:0 --require-auth >"$LOG_R" 2>&1 &
PID_R=$!
READY_R=""
for _ in $(seq 1 100); do
  READY_R="$(grep -m1 '^LITECODE_READY ' "$LOG_R" || true)"
  [[ -n "$READY_R" ]] && break
  sleep 0.1
done
[[ -n "$READY_R" ]] || { echo "restart failed" >&2; cat "$LOG_R" >&2; exit 1; }
BASE_R="$(echo "$READY_R" | sed -E 's/^LITECODE_READY (http:\/\/127\.0\.0\.1:[0-9]+\/?).*/\1/' | sed 's:/*$::')"
OPEN_CODE="$(curl -s -o /dev/null -w "%{http_code}" -X POST -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"path":"/tmp"}' "$BASE_R/api/workspace/""open" || true)"
[[ "$OPEN_CODE" == "404" || "$OPEN_CODE" == "405" ]] || {
  echo "expected workspace open route gone (404/405), got $OPEN_CODE" >&2
  exit 1
}

echo "==> smoke ok"
