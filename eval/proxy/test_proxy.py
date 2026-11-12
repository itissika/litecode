"""The egress proxy, against a fake upstream: no network, no key, no guessing."""

from __future__ import annotations

import http.client
import json
import shutil
import tempfile
import threading
import time
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import deepseek_proxy

KEY = "sk-test-secret"


class FakeUpstream(BaseHTTPRequestHandler):
    """Answers like the DeepSeek API for the shapes the proxy has to handle."""

    protocol_version = "HTTP/1.1"
    seen: list[dict] = []

    def log_message(self, format, *args):  # noqa: A002 - stdlib name
        return

    def do_POST(self):  # noqa: N802 - stdlib name
        length = int(self.headers.get("Content-Length") or 0)
        body = json.loads(self.rfile.read(length) or b"{}")
        FakeUpstream.seen.append(
            {"path": self.path, "headers": dict(self.headers), "body": body}
        )
        if body.get("model") == "boom" or self.path.endswith("/boom"):
            payload = {"error": {"message": "upstream exploded"}}
            data = json.dumps(payload).encode()
            self.send_response(500)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
            return
        if body.get("stream") is True:
            self._stream(body)
            return
        if body.get("model") == "no-usage":
            payload = {"id": "x", "choices": [{"message": {"content": "hi"}}]}
        else:
            payload = {
                "id": "x",
                "choices": [{"message": {"content": "hi"}}],
                "usage": {
                    "prompt_tokens": 7,
                    "completion_tokens": 2,
                    "total_tokens": 9,
                    "prompt_tokens_details": {"cached_tokens": 4},
                },
            }
        data = json.dumps(payload).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def _stream(self, body):
        asked = (body.get("stream_options") or {}).get("include_usage") is True
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Connection", "close")
        self.end_headers()
        chunk = {"id": "x", "choices": [{"index": 0, "delta": {"content": "hi"}}]}
        self.wfile.write(b"data: " + json.dumps(chunk).encode() + b"\n\n")
        self.wfile.flush()
        if asked and body.get("model") != "no-usage":
            usage = {
                "id": "x",
                "choices": [],
                "usage": {
                    "prompt_tokens": 11,
                    "completion_tokens": 5,
                    "total_tokens": 16,
                    "prompt_cache_hit_tokens": 3,
                },
            }
            self.wfile.write(b"data: " + json.dumps(usage).encode() + b"\n\n")
            self.wfile.flush()
        self.wfile.write(b"data: [DONE]\n\n")
        self.wfile.flush()
        self.close_connection = True


class ProxyTest(unittest.TestCase):
    def setUp(self) -> None:
        FakeUpstream.seen = []
        self.work = Path(tempfile.mkdtemp(prefix="proxy-test-"))
        self.addCleanup(shutil.rmtree, self.work, True)
        self.upstream = ThreadingHTTPServer(("127.0.0.1", 0), FakeUpstream)
        self.upstream.daemon_threads = True
        threading.Thread(target=self.upstream.serve_forever, daemon=True).start()
        self.addCleanup(self.upstream.server_close)
        self.addCleanup(self.upstream.shutdown)

        self.key = self.work / "key"
        self.key.write_text(KEY + "\n", encoding="utf-8")
        self.log = self.work / "requests.jsonl"
        proxy = deepseek_proxy.Proxy(
            f"http://127.0.0.1:{self.upstream.server_address[1]}",
            KEY.strip(),
            self.log,
            timeout=10,
        )
        self.server = deepseek_proxy.build_server("127.0.0.1", 0, proxy)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.addCleanup(self.server.server_close)
        self.addCleanup(self.server.shutdown)
        self.port = self.server.server_address[1]

    # ── helpers ──────────────────────────────────────────────────────────

    def call(self, path: str, payload: dict) -> tuple[int, bytes]:
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=10)
        data = json.dumps(payload).encode()
        connection.request(
            "POST",
            path,
            body=data,
            headers={"Content-Type": "application/json", "Authorization": "Bearer client"},
        )
        response = connection.getresponse()
        body = response.read()
        connection.close()
        return response.status, body

    def rows(self, expected: int = 1) -> list[dict]:
        """The JSONL so far, waiting for the handler thread to append its row."""
        deadline = time.monotonic() + 3
        while True:
            if self.log.exists():
                rows = [
                    json.loads(line)
                    for line in self.log.read_text(encoding="utf-8").splitlines()
                ]
            else:
                rows = []
            if len(rows) >= expected or time.monotonic() > deadline:
                return rows
            time.sleep(0.01)

    # ── tests ────────────────────────────────────────────────────────────

    def test_a_plain_request_is_forwarded_and_logged(self) -> None:
        status, body = self.call(
            "/r/task-1/v1/chat/completions",
            {"model": "deepseek-flash", "messages": [{"role": "user", "content": "hi"}]},
        )
        self.assertEqual(status, 200)
        self.assertEqual(json.loads(body)["choices"][0]["message"]["content"], "hi")

        # The upstream saw the run's path stripped and the real key, not the client's.
        self.assertEqual(FakeUpstream.seen[0]["path"], "/v1/chat/completions")
        self.assertEqual(FakeUpstream.seen[0]["headers"]["Authorization"], f"Bearer {KEY}")

        row = self.rows()[0]
        self.assertEqual(row["run_id"], "task-1")
        self.assertEqual(row["model"], "deepseek-flash")
        self.assertEqual(row["status"], 200)
        self.assertFalse(row["stream"])
        self.assertEqual(row["usage"], "ok")
        self.assertEqual(
            (row["prompt_tokens"], row["completion_tokens"], row["cached_tokens"]),
            (7, 2, 4),
        )
        self.assertGreaterEqual(row["duration_ms"], 0)

    def test_a_stream_without_usage_asks_for_it_and_swallows_the_frame(self) -> None:
        status, body = self.call(
            "/r/task-2/v1/chat/completions",
            {"model": "deepseek-flash", "stream": True, "messages": []},
        )
        self.assertEqual(status, 200)
        text = body.decode()
        self.assertIn('"content": "hi"', text)
        self.assertIn("[DONE]", text)
        # The client asked for no usage, so it must not suddenly receive one.
        self.assertNotIn('"usage"', text)

        forwarded = FakeUpstream.seen[0]["body"]
        self.assertIs(forwarded["stream_options"]["include_usage"], True)

        row = self.rows()[0]
        self.assertTrue(row["stream"])
        self.assertTrue(row["usage_injected"])
        self.assertEqual(row["usage"], "ok")
        self.assertEqual(
            (row["prompt_tokens"], row["completion_tokens"], row["cached_tokens"]),
            (11, 5, 3),
        )

    def test_a_stream_that_reports_no_usage_is_marked_missing(self) -> None:
        status, body = self.call(
            "/r/task-3/v1/chat/completions",
            {"model": "no-usage", "stream": True, "messages": []},
        )
        self.assertEqual(status, 200)
        self.assertIn("[DONE]", body.decode())
        row = self.rows()[0]
        self.assertEqual(row["usage"], "missing")
        self.assertIsNone(row["prompt_tokens"])
        self.assertIsNone(row["completion_tokens"])
        self.assertIsNone(row["cached_tokens"])
        self.assertIn("no usage", row["error"])

    def test_an_upstream_failure_is_logged_with_its_status(self) -> None:
        status, _body = self.call(
            "/r/task-4/v1/chat/completions", {"model": "boom", "messages": []}
        )
        self.assertEqual(status, 500)
        row = self.rows()[0]
        self.assertEqual(row["status"], 500)
        self.assertEqual(row["usage"], "missing")
        self.assertIn("no usage", row["error"])

    def test_a_models_probe_is_logged_as_not_billable(self) -> None:
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=10)
        connection.request("GET", "/r/task-5/v1/models")
        response = connection.getresponse()
        response.read()
        connection.close()
        row = self.rows()[0]
        self.assertEqual(row["usage"], "n/a")
        self.assertIsNone(row["prompt_tokens"])

    def test_a_path_without_a_run_id_is_refused(self) -> None:
        status, _body = self.call("/v1/chat/completions", {"model": "x"})
        self.assertEqual(status, 404)
        self.assertEqual(self.rows(), [])

    def test_a_missing_or_empty_key_file_stops_startup(self) -> None:
        missing = self.work / "nope"
        code = deepseek_proxy.main(
            ["--key-file", str(missing), "--log", str(self.work / "rows.jsonl")]
        )
        self.assertEqual(code, 2)

        empty = self.work / "empty"
        empty.write_text("  \n", encoding="utf-8")
        code = deepseek_proxy.main(["--key-file", str(empty), "--log", str(self.work / "rows.jsonl")])
        self.assertEqual(code, 2)


if __name__ == "__main__":
    unittest.main()
