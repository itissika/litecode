# Live wire (LITECODE_LLM_WIRE) — unavailable on diagnostic VM

- **Date:** 2026-10-15 (WP-178 / 175-a)
- **Reason:** No Command Code / Luna API credentials in this cloud agent environment; `LITECODE_LLM_WIRE` was not exercised against production.
- **Substitute evidence:** Synthetic SSE fixtures under `sse-fixtures/` and the read-only replay binary `h1-verify/` (mirrors `ResponsesCodec::parse_stream_event` + Luna catalog `usage_patch = map_max_effort_to_xhigh`).
- **Follow-up for provider:** On a machine with Luna access, run one tool-capable turn with `LITECODE_LLM_WIRE=<dir>` (or `serve.sh --wire`) and compare failing SSE `data:` lines to fixtures B/C here.
