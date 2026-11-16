# Four-harness bench (Harbor 0.25.0)

One panel, 32 cells: `litecode`, `codex`, `pi`, `opencode` ×
`prove-plus-comm`, `polyglot-c-py`, `password-recovery`, `cancel-async-tasks`,
`path-tracing`, `regex-chess`, `write-compressor`,
`feal-differential-cryptanalysis`, all on `deepseek/deepseek-flash` at the
official DeepSeek endpoint. Task images stay the public task images; the
harness CLIs come from one shared runtime.

LiteCode's scored terminal surface now mirrors Codex's unified exec:
`process.exec_command` starts a pipe/PTY session and yields either an exit or a
live `session_id`; `process.write_stdin` polls output, feeds stdin, and maps
`"\u0003"` to process-group SIGINT. The legacy `run` / `wait` / `kill` trio is
still bound for compatibility but is documented as the non-interactive path.

## Why the pieces look like this

| Problem | Decision |
|---|---|
| Harness installs dominated trial time and leaked registry flakiness into scores | Prebuild one shared runtime (Node 22 + codex/pi/opencode + rg) and bind-mount it read-only at `/opt/harness`; `prebaked_agents.py` writes a PATH shim and verifies the binary. Task images stay raw. |
| Per-task toolchains duplicated ~2.2GB per task | The runtime is stored once and mounted into every trial; adding a task costs only its public image. |
| A job gives every agent one fixed `base_url`, so three tasks in one job would merge into one bill per harness | One Harbor job per task: `bench5-<task>-<attempt>`. The proxy run id is `<job>-<harness>`, so every cell has its own token ledger. |
| Harness self-report is not comparable, and opencode omits thinking from its output count | LiteCode uses its own final usage line (the only self-report we trust); the other three are measured only at the egress proxy. Self-report is retained as a cross-check. |
| Trajectories were missing or unreadable | Every adapter writes ATIF `agent/trajectory.json`; `collect.py` summarizes steps/tool calls and keeps the raw file in the trial. |
| Infra failures were being averaged into pass rates | A trial with no verifier result or an exception is retried; it is not a scored failure and not a zero-token cell. |
| Cost tables differ across harnesses | One price table (`$0.15/$0.003/$0.60 per M`) is applied to every trusted token count. |

## Run order

All commands run in WSL, where Docker lives. Examples use `$HOME` for local state; run them from the repository root.

```bash
# 1. LiteCode binary, clean HEAD of the sparse build checkout.
#    PORTABLE=1 builds on Debian 12 with GLIBC_2.35 and ORT load-dynamic,
#    so the same binary runs on Debian 12 and Ubuntu 24.04 task images.
cd ~/build/litecode-src && LITECODE_EVAL_PORTABLE=1 bash eval/harbor/build.sh

# 2. Shared harness runtime (Node + codex/pi/opencode; no task images)
cd /path/to/litecode && bash eval/harbor/runtime/build.sh

# 3. Proxy: one accounting exit, writing this campaign's exact egress_log
cd /path/to/litecode
python3 eval/harbor/bench5/start_proxy.py \
  --campaign eval/harbor/bench5/campaigns/bench5.yaml

# 4. No-cost gate: images, disk, proxy, synthetic usage=ok, dataset checksums
python3 eval/harbor/bench5/run_matrix.py \
  --campaign eval/harbor/bench5/campaigns/bench5.yaml --preflight-only

# 5. Panel
export PATH="$HOME/.local/bin:$PATH"
python3 eval/harbor/bench5/run_matrix.py \
  --campaign eval/harbor/bench5/campaigns/bench5.yaml --tag r1

# 6. Report (reads only finished jobs + egress JSONL)
python3 eval/harbor/bench5/collect.py \
  --campaign eval/harbor/bench5/campaigns/bench5.yaml
```

`run_matrix.py` copies the campaign task directories to the campaign's
`dataset` path, rewrites only `environment.docker_image` to the campaign image,
and records the source checksum and both image names in `manifest.json`.
Instructions, tests, solution and timeouts stay untouched.

## What a row means

* **Requests**: chat/response generation calls, from the proxy for the three
  proxied harnesses; from the ATIF trajectory (or LiteCode's agent-loop log) for
  LiteCode.
* **prompt**: total input tokens including cache hits.
* **cache**: `cached / prompt`.
* **output**: completion tokens including reasoning where the provider reports
  it.
* **cost**: recomputed from the three prices above, not copied from a harness.
* **agent time**: `agent_execution` window only; environment/setup and verifier
  time are separate and kept in the JSON.

## Outputs

* `~/eval/bench5-report.md` — the human table and conclusions.
* `~/eval/bench5-report.json` — every selected cell, its egress request series,
  artifact inventory, trajectory summary, both token sources and cost breakdown.
  LiteCode cells also carry `tool_usage`: per-tool calls, input keys, output
  fields, duration/bytes and classified up-float reminders.
* `agent/litecode/diagnostics/<session>/tool-audit.jsonl` — the raw tool-layer
  span file copied from the task container. One `start` and one `exec_done`
  row per call, joined by `call_id`; it contains names, not argument values.
* `~/eval/jobs/bench5/registry/attempts.jsonl` — every attempt, including
  repairs.
* `~/eval/jobs/bench5/registry/selected.json` — the cell → trial mapping the
  report used, including `rankable`, token totals, cost and audit warnings.
* `~/eval/jobs/bench5/registry/audit.md` / `audit.json` — the runner's final
  strict-audit report when a full campaign finishes.
* `~/eval/jobs/bench5/campaign.json` — the campaign claimed this jobs root,
  with expected cells and the strict policy.

## Native trace, not ATIF reasoning

`agent/trajectory.json` is a normalized, lossy view: it keeps tool calls and
observations, but several adapters drop reasoning text. The native logs are the
source of truth:

| harness | native trace |
|---|---|
| LiteCode | `agent/litecode/sessions.db`, `logs/litecode.log`, `diagnostics/*/tool-audit.jsonl` |
| Codex | `agent/sessions/**/rollout-*.jsonl` |
| Pi | `agent/pi/sessions/*.jsonl` |
| Opencode | `agent/trajectory.json` (`reasoning_content` is in the steps) |

`trace.py` normalizes all four into one step schema and a summary with
`steps`, `requests`, `prompt`, `output`, `reasoning`, `polls`, `timeouts`,
`live_sessions`, `clamped`, `errors`, `wall_time_s` and `agent_sec`.
`run_turn.sh` now copies `.litecode/blobs` next to the session database; a
trial that predates that copy is reported as `reasoning_complete=false`, never
as zero reasoning.

```bash
python3 eval/harbor/bench5/trace.py summary <trial-dir> --harness litecode
python3 eval/harbor/bench5/trace.py steps <trial-dir> --harness codex --step 23
python3 eval/harbor/bench5/trace.py compare litecode=<trial> codex=<trial> pi=<trial> opencode=<trial>
python3 eval/harbor/bench5/trace.py scan ~/eval/jobs/bench5 --task regex-chess
```

The summary carries the data-quality flags that the aggregate table must show:

* `reasoning_source`: native source, or `missing`.
* `reasoning_complete` / `reasoning_missing_blobs`: a missing LiteCode blob is
  reported, never filled with 0.
* `completion_includes_reasoning`: Opencode reports completion without
  reasoning; `output_tokens_total` adds it back for a fair output comparison.
* `steps_without_reasoning`: steps that emitted no reasoning, which is not the
  same as a capture loss.
* `wall_time_s` is tool-call wall time; `agent_sec` is the agent execution
  window. Do not compare one as the other.

The collector now writes the same summary into each cell (`trace`) and into
the top-level report (`traces`), so the next report cannot silently rank a
harness whose reasoning text was never captured.

## Retry semantics

`run_matrix.py --retry-rounds 2` reruns only harnesses whose cell did not reach
a verifier result. A legitimate `reward: 0.0` is kept as a scored failure.
The report selects the latest *valid* attempt per cell; an older valid attempt
is never overwritten by a newer infrastructure failure.
## Campaign manifest: the expansion surface

`campaigns/bench5.yaml` is now the source of truth for the matrix. Runner and
collector both read it; there is no `TASKS = (...)` tuple to forget.

A campaign declares:

* `tasks`: name, public `image`, optional `source_image`, difficulty;
* `harnesses`: id and token source (`litecode` or `egress`);
* `harness_runtime`: the shared directory mounted at `/opt/harness`;
* `jobs_root`, `dataset`, `source_dataset`, `egress_log`, `egress_root`;
* `concurrency`, `retry_rounds`, `job_timeout_sec`, `min_free_gb`;
* `price_usd_per_million`: the versioned price table used by the report;
* `strict`: expected-cell, token-completeness, trajectory and clean-exit gates.

`campaign.expected_cells` is the task x harness product. The collector refuses
to return success when a cell is missing or not rankable. A new task can no
longer be silently filtered out.

## Adding a task

1. Put the task under `source_dataset` (`~/tb2` by default).
2. Add the task entry with its public `image` and `source_image`; the runner
   checks that `task.toml` still names that source image. No image build is
   needed.
3. Run the no-cost gate:

   ```bash
   python3 eval/harbor/bench5/run_matrix.py \
     --campaign eval/harbor/bench5/campaigns/bench5.yaml --preflight-only
   ```

   This checks Harbor 0.25.x, disk space, the shared runtime on both supported
   glibc families, task-image availability (local or registry), the LiteCode
   binary/provenance, the proxy `/healthz`, and a tiny synthetic generation
   whose `usage=ok` row must land in the campaign's `egress_log`. It also
   prepares the dataset copy and records source-tree checksums.
4. Run the panel. At the end the runner invokes the collector's strict audit;
   a missing or unrankable cell makes the runner exit non-zero.

## Campaign isolation

Use a distinct `jobs_root` and `egress_log` per campaign. The proxy must be
started with a mount that writes that exact log; the synthetic preflight call
proves the path before any scored request. A reused `--tag` is not silently
mixed into an old campaign: run ids are generated from the campaign prefix and
tag, and the collector only reads the campaign's `jobs_root`.

## Strict gates

* `require_expected_cells`: every task x harness key must be present.
* `require_token_complete`: non-LiteCode needs `usage=ok` on generation rows;
  LiteCode needs `agent_result` and `litecode.txt` to agree. Failed/aborted
  auxiliary requests are recorded as warnings, not billed, and do not hide a
  successful generation with no usage.
* `require_trajectory`: every selected cell needs a parseable ATIF file.
* `require_clean_exit`: optional; verifier-passing non-zero exits are shown as
  `通过（非零退出）` and are rankable unless this is set true.

## What is still outside the gate

The shared runtime assumes the current supported base: glibc (Ubuntu 24.04 /
Debian 12), x86_64, and a task image with `bash`, `grep`, `stdbuf`, `tee` and
normally `git`. Alpine/musl, ARM and non-root images need a separate runtime or
adapter; the runtime preflight rejects an incompatible mount rather than
letting it become a missing trial. Each task is still one trial; pass@k and
difficulty stratification remain a separate evaluation decision.
The runner also writes `jobs_root/campaign.json`; a second campaign with a
different name cannot claim the same jobs root. Reusing an existing run id /
tag is refused unless `--allow-reuse-tag` is passed explicitly. The runner pulls
a missing task image with `docker pull`: the Docker daemon needs registry
access even though agent traffic goes through the egress proxy.
A filtered repair (`--task` / `--harness`) skips the whole-campaign strict
audit and prints a reminder; run the collector over the full campaign after
the repair so `selected.json` and the report reflect the complete matrix.