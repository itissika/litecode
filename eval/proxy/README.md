# One accounting exit

Every harness in this eval points its OpenAI-compatible base_url at
`http://<host>:<port>/r/<run_id>/v1`. `deepseek_proxy.py` strips the run id,
forwards the request to the upstream with the real key, and appends one JSONL row
per request. The run id is what ties a request to a task, so a job's whole bill
is one grep away.

## Running it

Run the proxy **inside a container**, not on the WSL host: the host has no
outbound network, so a proxy there makes every harness request time out. Publish
the port and keep the key and the log on mounts:

```bash
docker run -d --name litecode-egress -p 8399:8399 \
  -v <repo>/eval/proxy:/p:ro \
  -v ~/eval/deepseek_key:/k/deepseek_key:ro \
  -v ~/eval/logs:/logs \
  python:3.11 python /p/deepseek_proxy.py \
    --host 0.0.0.0 --port 8399 \
    --key-file /k/deepseek_key \
    --log /logs/egress.jsonl
```

Verified from another container: `172.17.0.1:8399` reaches this proxy, and
non-stream, stream-without-usage, and stream-with-usage all land in the JSONL
as `"usage": "ok"`.

* `--host` defaults to `127.0.0.1`; a container run needs `--host 0.0.0.0`.
  Never expose the port beyond the host's own interfaces.
* The key is read from a file at startup, mounted read-only. It is never an
  argument, never logged, and must not live in this repository.
* `--log` must be outside the repository; nothing appends to it but this process.
* Base url for a harness: `http://172.17.0.1:8399/r/<run_id>/v1`, never
  `localhost` — inside a container that name is the container itself.

## What a row says

`{"ts", "request_id", "run_id", "method", "path", "stream", "usage_injected",
"model", "status", "duration_ms", "prompt_tokens", "completion_tokens",
"cached_tokens", "total_tokens", "usage", "error"}`

* `usage` is `ok` when the upstream reported token counts, `missing` when a
  generation call came back without any — and then the token fields are `null`.
  Nothing is estimated. A path that is not a generation (`/v1/models`) says
  `n/a`, so a probe never looks like a failed measurement.
* `cached_tokens` comes from `prompt_cache_hit_tokens` (DeepSeek) or
  `prompt_tokens_details.cached_tokens` (OpenAI shape); a Responses payload
  reports `input_tokens` / `output_tokens` / `input_tokens_details.cached_tokens`
  and is read the same way.
* `duration_ms` is wall time from request received to response finished.

## Streaming

When a client asks for a stream without `stream_options.include_usage`, the
proxy adds it on the way out and swallows the frame that only carries usage on
the way back: the client gets the same event shapes it asked for, the row still
has the tokens. A client that asks for usage itself gets that frame untouched.

`/responses` (what Codex speaks) is not that stream: it takes no
`stream_options`, so nothing is injected and nothing is swallowed. Its usage
rides on the final `response.completed` event, under `response.usage`, and the
row reads it from there; every event is forwarded as it arrives.

## Tests

```bash
cd eval/proxy && python -m unittest test_proxy
```

Nine tests against a fake upstream: forwarding and the key swap, the swallowed
usage frame, a stream with no usage (`"usage": "missing"`), an upstream 500, a
`/v1/models` probe, a path without a run id, a missing or empty key file, and
two Responses calls (streamed and not) whose usage is read from
`response.completed` / the top-level body.
