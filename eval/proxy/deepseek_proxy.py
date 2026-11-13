#!/usr/bin/env python3
"""One accounting exit for every harness: an OpenAI-compatible forwarder.

Every eval run points its base_url at `http://<host>:<port>/r/<run_id>/v1`. The
proxy strips the run id, forwards the request to the upstream API with the real
key, and appends one JSONL row per request: run id, time, model, prompt /
completion / cached tokens, duration, status, and whether it streamed.

The run id is the only thing that ties a request to a task, so a whole job's
bill is `@grep '"run_id":"<job>"'`-able without joining anything.

Streaming: every streamed chat request leaves with
`stream_options.include_usage = true`, merged into whatever the client sent
(even a client-sent `false`). When the client did not ask for usage itself, the
frame that carries it is swallowed on the way back — the client never sees a chunk shape it did not ask for, while the row
still records the tokens. A response with no usage records nulls and
`"usage": "missing"`; nothing is estimated.

Responses (`/responses`) is a different stream: the request has no
`stream_options` field, so nothing is injected and nothing is swallowed; the
usage rides on the final `response.completed` event, under `response.usage`.

The key is read from a file at startup, never from argv, never logged, and not
kept in this repository.
"""

from __future__ import annotations

import argparse
import http.client
import json
import os
import re
import sys
import threading
import time
import uuid
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any
from urllib.parse import urlsplit

# /r/<run_id>/v1/... -> ("<run_id>", "/v1/..."); the rest of the path is the API.
_RUN_PATH = re.compile(r"^/r/(?P<run>[A-Za-z0-9._-]{1,64})(?P<rest>/.*)$")

# Paths whose response is a billable generation; everything else (a /v1/models
# probe) has no usage to miss, so its row says so instead of "missing".
_BILLABLE = re.compile(r"/(chat/completions|completions|responses)$")

# Only these request bodies take `stream_options.include_usage`; sending it to
# /responses would be a 400, and Responses reports usage without being asked.
_USAGE_INJECTION_PATHS = ("/chat/completions", "/completions")

# Roles are a chat-completions concept; a Responses input item has no role to
# rewrite. pi opens with role="developer", which the upstream rejects outright,
# so the one rewrite this proxy performs stays inside this path.
_CHAT_COMPLETIONS_PATH = "/chat/completions"
_ROLE_REWRITES = {"developer": "system"}

# Headers that belong to this hop, not to the upstream request.
_HOP_HEADERS = {
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailers",
    "transfer-encoding",
    "upgrade",
    "host",
    "content-length",
    "authorization",
    # Accounting parses the body to find usage, so it must be the plain body:
    # a harness that advertises gzip would otherwise hand us bytes we cannot
    # read (and would explain a "carried no usage" on a healthy response).
    "accept-encoding",
}

_LOG_LOCK = threading.Lock()


def _rewrite_roles(body: dict[str, Any]) -> list[str]:
    """Rewrite roles the upstream rejects, and say what was rewritten.

    Mutates *body* in place and returns the distinct rewrites performed, so the
    JSONL row can show them instead of the change staying invisible.
    """
    messages = body.get("messages")
    if not isinstance(messages, list):
        return []
    rewrites: list[str] = []
    for message in messages:
        if not isinstance(message, dict):
            continue
        role = message.get("role")
        if isinstance(role, str) and role in _ROLE_REWRITES:
            message["role"] = _ROLE_REWRITES[role]
            rewrites.append(f"{role}->{_ROLE_REWRITES[role]}")
    return list(dict.fromkeys(rewrites))


def _now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")


def _counts_from_usage(usage: Any) -> dict[str, int] | None:
    """Token counts from a Chat-Completions or Responses usage object."""
    if not isinstance(usage, dict):
        return None
    prompt = usage.get("prompt_tokens")
    completion = usage.get("completion_tokens")
    if not isinstance(prompt, int) or not isinstance(completion, int):
        # Responses spells the same two counts input_tokens / output_tokens.
        prompt = usage.get("input_tokens")
        completion = usage.get("output_tokens")
    if not isinstance(prompt, int) or not isinstance(completion, int):
        return None
    cached = usage.get("prompt_cache_hit_tokens")
    if not isinstance(cached, int):
        for name in ("prompt_tokens_details", "input_tokens_details"):
            details = usage.get(name)
            if isinstance(details, dict) and isinstance(details.get("cached_tokens"), int):
                cached = details["cached_tokens"]
                break
    record = {
        "prompt_tokens": prompt,
        "completion_tokens": completion,
        "cached_tokens": cached if isinstance(cached, int) else None,
    }
    total = usage.get("total_tokens")
    if isinstance(total, int):
        record["total_tokens"] = total
    return record


def _usage_from(payload: Any) -> dict[str, int] | None:
    """Token counts from a JSON body or a parsed SSE event, or None."""
    if not isinstance(payload, dict):
        return None
    counts = _counts_from_usage(payload.get("usage"))
    if counts is not None:
        return counts
    # A Responses event carries the response object one level down.
    response = payload.get("response")
    if isinstance(response, dict):
        return _counts_from_usage(response.get("usage"))
    return None


class Proxy:
    """Config plus the one writable log. Shared by every request thread."""

    def __init__(self, upstream: str, key: str, log: Path, timeout: float) -> None:
        parsed = urlsplit(upstream.rstrip("/"))
        if parsed.scheme not in ("http", "https") or not parsed.hostname:
            raise ValueError(f"upstream must be http(s)://host[:port], got {upstream!r}")
        self.scheme = parsed.scheme
        self.host = parsed.hostname
        self.port = parsed.port or (443 if parsed.scheme == "https" else 80)
        self.prefix = parsed.path.rstrip("/")
        self.upstream = upstream
        self.key = key
        self.log = log
        self.timeout = timeout

    def connection(self) -> http.client.HTTPConnection:
        if self.scheme == "https":
            return http.client.HTTPSConnection(self.host, self.port, timeout=self.timeout)
        return http.client.HTTPConnection(self.host, self.port, timeout=self.timeout)

    def write(self, row: dict[str, Any]) -> None:
        line = json.dumps(row, ensure_ascii=False, sort_keys=True) + "\n"
        with _LOG_LOCK:
            with self.log.open("a", encoding="utf-8") as handle:
                handle.write(line)
                handle.flush()
                os.fsync(handle.fileno())


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "litecode-egress/1"
    proxy: Proxy  # set on the subclass the server serves

    # ── plumbing ─────────────────────────────────────────────────────────

    def log_message(self, format: str, *args: Any) -> None:  # noqa: A002 - stdlib name
        # One line per request goes to the JSONL, not to stderr.
        return

    def _fail(self, status: int, message: str) -> None:
        body = json.dumps({"error": {"message": message, "type": "proxy_error"}}).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
        self.wfile.flush()

    def do_GET(self) -> None:  # noqa: N802 - stdlib name
        if self.path == "/healthz":
            body = json.dumps(
                {"ok": True, "upstream": self.proxy.upstream}
            ).encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            self.wfile.flush()
            return
        self._proxy_request()

    def do_POST(self) -> None:  # noqa: N802 - stdlib name
        self._proxy_request()

    # ── one request ──────────────────────────────────────────────────────

    def _proxy_request(self) -> None:
        match = _RUN_PATH.match(self.path)
        if match is None:
            self._fail(404, "path must be /r/<run_id>/v1/...")
            return
        run_id = match.group("run")
        rest = self.proxy.prefix + match.group("rest")

        length = self.headers.get("Content-Length")
        raw = self.rfile.read(int(length)) if length else b""
        try:
            body = json.loads(raw) if raw else None
        except ValueError:
            body = None

        adapted: list[str] = []
        if rest.endswith(_CHAT_COMPLETIONS_PATH) and isinstance(body, dict):
            adapted = _rewrite_roles(body)
            if adapted:
                raw = json.dumps(body).encode("utf-8")

        injected = False
        client_asked_usage = False
        wants_usage = rest.endswith(_USAGE_INJECTION_PATHS)
        if wants_usage and isinstance(body, dict) and body.get("stream") is True:
            # A streamed chat request always goes upstream with include_usage on,
            # whatever the client put there: seeing a stream_options key is not
            # the same as asking for usage, and a harness that sends
            # include_usage=false still deserves a counted run.
            options = body.get("stream_options")
            client_asked_usage = (
                isinstance(options, dict) and options.get("include_usage") is True
            )
            merged = dict(options) if isinstance(options, dict) else {}
            merged["include_usage"] = True
            if merged != options:
                body = {**body, "stream_options": merged}
                injected = True
                raw = json.dumps(body).encode("utf-8")

        billable = _BILLABLE.search(rest) is not None
        # Request shape is trace data, not billing: it explains why one harness
        # re-sends more context without storing prompts or tool output.
        messages = body.get("messages") if isinstance(body, dict) else None
        input_items = body.get("input") if isinstance(body, dict) else None
        tools = body.get("tools") if isinstance(body, dict) else None
        row: dict[str, Any] = {
            "ts": _now(),
            "request_id": uuid.uuid4().hex,
            "run_id": run_id,
            "method": self.command,
            "path": rest,
            "stream": bool(isinstance(body, dict) and body.get("stream") is True),
            "usage_injected": injected,
            "adapted_roles": adapted or None,
            "client_asked_usage": client_asked_usage,
            "stream_events": None,
            "model": body.get("model") if isinstance(body, dict) else None,
            "request_bytes": len(raw),
            "messages_count": len(messages) if isinstance(messages, list) else None,
            "input_items_count": len(input_items) if isinstance(input_items, list) else None,
            "tools_count": len(tools) if isinstance(tools, list) else None,
            "response_bytes": None,
            "status": None,
            "duration_ms": None,
            "prompt_tokens": None,
            "completion_tokens": None,
            "cached_tokens": None,
            "total_tokens": None,
            "usage": "missing" if billable else "n/a",
            "error": None,
        }
        started = time.monotonic()
        connection = self.proxy.connection()
        try:
            connection.putrequest(self.command, rest, skip_accept_encoding=True)
            for name, value in self.headers.items():
                if name.lower() in _HOP_HEADERS:
                    continue
                connection.putheader(name, value)
            connection.putheader("Authorization", f"Bearer {self.proxy.key}")
            connection.putheader("Accept-Encoding", "identity")
            connection.putheader("Content-Length", str(len(raw)))
            connection.endheaders(raw)
            response = connection.getresponse()
            row["status"] = response.status
            self._response_bytes = 0
            streaming = row["stream"]
            if streaming:
                usage, events = self._relay_stream(response, injected=injected)
                row["stream_events"] = events
            else:
                usage = self._relay_whole(response)
            if usage is not None:
                row.update(usage)
                row["usage"] = "ok"
            elif billable:
                row["usage"] = "missing"
                row["error"] = f"upstream {response.status} carried no usage"
        except Exception as error:  # noqa: BLE001 - the row must record it
            row["error"] = f"{type(error).__name__}: {error}"
            if row["status"] is None:
                row["status"] = 502
            if not self.wfile.closed:
                try:
                    self._fail(502, f"upstream failed: {error}")
                except OSError:
                    pass
        finally:
            connection.close()
            row["response_bytes"] = getattr(self, "_response_bytes", None)
            row["duration_ms"] = round((time.monotonic() - started) * 1000, 1)
            self.proxy.write(row)

    # ── response paths ───────────────────────────────────────────────────

    def _relay_whole(self, response: http.client.HTTPResponse) -> dict[str, int] | None:
        data = response.read()
        self._response_bytes = len(data)
        self.send_response(response.status)
        self._copy_headers(response)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)
        self.wfile.flush()
        try:
            payload = json.loads(data)
        except ValueError:
            return None
        return _usage_from(payload)

    def _relay_stream(
        self, response: http.client.HTTPResponse, *, injected: bool
    ) -> tuple[dict[str, int] | None, int]:
        """Forward events as they arrive; swallow the usage frame we asked for."""
        self._swallowed_usage = None
        events = 0
        self.send_response(response.status)
        self._copy_headers(response)
        self.send_header("Connection", "close")
        self.end_headers()
        usage: dict[str, int] | None = None
        event: list[bytes] = []
        while True:
            line = response.readline()
            if not line:
                break
            self._response_bytes += len(line)
            if line in (b"\n", b"\r\n"):
                events += 1
                if self._keep_event(event, injected=injected):
                    for buffered in event + [line]:
                        self.wfile.write(buffered)
                    self.wfile.flush()
                event = []
                continue
            event.append(line)
        if event:  # a final event without its blank line
            events += 1
            if self._keep_event(event, injected=injected):
                self.wfile.write(b"".join(event))
                self.wfile.flush()
        usage_frame = getattr(self, "_swallowed_usage", None)
        if usage_frame is not None:
            usage = usage_frame
        self.close_connection = True
        return usage, events

    def _keep_event(self, event: list[bytes], *, injected: bool) -> bool:
        """Record the usage of one SSE event; say whether the client gets it."""
        payload = b""
        for line in event:
            if line.startswith(b"data:"):
                payload += line[len(b"data:") :].strip()
        if not payload or payload == b"[DONE]":
            return True
        try:
            parsed = json.loads(payload)
        except ValueError:
            return True
        usage = _usage_from(parsed)
        if usage is None:
            return True
        self._swallowed_usage = usage
        choices = parsed.get("choices")
        # The frame that only carries usage has no choices; a client that asked
        # for usage itself expects it, so only a frame we asked for is dropped.
        return not (injected and choices == [])

    def _copy_headers(self, response: http.client.HTTPResponse) -> None:
        for name, value in response.getheaders():
            if name.lower() in {
                "connection",
                "transfer-encoding",
                "content-length",
                "keep-alive",
            }:
                continue
            self.send_header(name, value)


class Server(ThreadingHTTPServer):
    daemon_threads = True


def build_server(host: str, port: int, proxy: Proxy) -> Server:
    handler = type("BoundHandler", (Handler,), {"proxy": proxy})
    return Server((host, port), handler)


def main(argv: list[str] | None = None) -> int:
    description = (__doc__ or "").split("\n\n")[0]
    parser = argparse.ArgumentParser(description=description)
    parser.add_argument(
        "--host",
        default="127.0.0.1",
        help="interface to listen on; containers reach the host through the docker "
        "bridge address, so a container run needs --host 0.0.0.0 (or 172.17.0.1)",
    )
    parser.add_argument("--port", type=int, default=8080)
    parser.add_argument("--upstream", default="https://api.deepseek.com")
    parser.add_argument("--key-file", required=True, help="file holding the upstream API key")
    parser.add_argument(
        "--log", required=True, help="JSONL path; keep it out of the repository"
    )
    parser.add_argument("--timeout", type=float, default=600.0, help="upstream timeout, seconds")
    args = parser.parse_args(argv)

    key_path = Path(args.key_file)
    try:
        key = key_path.read_text(encoding="utf-8").strip()
    except OSError as error:
        print(f"proxy: cannot read {key_path}: {error}", file=sys.stderr)
        return 2
    if not key:
        print(f"proxy: {key_path} is empty", file=sys.stderr)
        return 2
    log_path = Path(args.log)
    if not log_path.parent.exists():
        print(f"proxy: log directory {log_path.parent} does not exist", file=sys.stderr)
        return 2

    proxy = Proxy(args.upstream, key, log_path, args.timeout)
    server = build_server(args.host, args.port, proxy)
    host, port = server.server_address[:2]
    print(
        f"proxy: listening on {host}:{port}, forwarding to {proxy.scheme}://"
        f"{proxy.host}:{proxy.port}, log {log_path}",
        file=sys.stderr,
    )
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
