# Five-harness bench (Harbor 0.25.0)

One panel, 20 cells: `litecode`, `codex`, `pi`, `opencode`, `grok-build` ×
`prove-plus-comm`, `polyglot-c-py`, `password-recovery`, `cancel-async-tasks`,
all on `deepseek/deepseek-flash` at the official DeepSeek endpoint.

LiteCode's scored terminal surface now mirrors Codex's unified exec:
`process.exec_command` starts a pipe/PTY session and yields either an exit or a
live `session_id`; `process.write_stdin` polls output, feeds stdin, and maps
`"\u0003"` to process-group SIGINT. The legacy `run` / `wait` / `kill` trio is
still bound for compatibility but is documented as the non-interactive path.

## Why the pieces look like this

| Problem | Decision |
|---|---|
| Harness installs dominated trial time and leaked registry flakiness into scores | Prebake Node 22 + codex + pi + opencode + grok-build into each task image; `prebaked_agents.py` verifies the binary and skips the installer. |
| A job gives every agent one fixed `base_url`, so three tasks in one job would merge into one bill per harness | One Harbor job per task: `bench5-<task>-<attempt>`. The proxy run id is `<job>-<harness>`, so every cell has its own token ledger. |
| Harness self-report is not comparable, and opencode omits thinking from its output count | LiteCode uses its own final usage line (the only self-report we trust); the other four are measured only at the egress proxy. Self-report is retained as a cross-check. |
| Trajectories were missing or unreadable | Every adapter writes ATIF `agent/trajectory.json`; `collect.py` summarizes steps/tool calls and keeps the raw file in the trial. |
| Infra failures were being averaged into pass rates | A trial with no verifier result or an exception is retried; it is not a scored failure and not a zero-token cell. |
| Cost tables differ across harnesses | One price table (`$0.15/$0.003/$0.60 per M`) is applied to every trusted token count. |

## Run order

All commands run in WSL, where Docker lives. Examples use `$HOME` for local state; run them from the repository root.

```bash
# 1. LiteCode binary, clean HEAD of the sparse build checkout.\n#    PORTABLE=1 builds on Debian 12 with GLIBC_2.35 and ORT load-dynamic,\n#    so the same binary runs on Debian 12 and Ubuntu 24.04 task images.
cd ~/build/litecode-src && LITECODE_EVAL_PORTABLE=1 bash eval/harbor/build.sh

# 2. Per-task eval images (thin grok layer on the verified toolchain images)
cd /path/to/litecode && bash eval/harbor/prebake/build.sh

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
`dataset` path, rewrites only `environment.docker_image` to the eval image, and
records the source checksum and both image names in `manifest.json`.
Instructions, tests, solution and timeouts stay untouched.

## What a row means

* **Requests**: chat/response generation calls, from the proxy for the four
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

## Retry semantics

`run_matrix.py --retry-rounds 2` reruns only harnesses whose cell did not reach
a verifier result. A legitimate `reward: 0.0` is kept as a scored failure.
The report selects the latest *valid* attempt per cell; an older valid attempt
is never overwritten by a newer infrastructure failure.
## Campaign manifest: the expansion surface

`campaigns/bench5.yaml` is now the source of truth for the matrix. Runner and
collector both read it; there is no `TASKS = (...)` tuple to forget.

A campaign declares:

* `tasks`: name, eval `image`, optional public `source_image`, difficulty;
* `harnesses`: id and token source (`litecode` or `egress`);
* `jobs_root`, `dataset`, `source_dataset`, `egress_log`, `egress_root`;
* `concurrency`, `retry_rounds`, `job_timeout_sec`, `min_free_gb`;
* `price_usd_per_million`: the versioned price table used by the report;
* `strict`: expected-cell, token-completeness, trajectory and clean-exit gates.

`campaign.expected_cells` is the task x harness product. The collector refuses
to return success when a cell is missing or not rankable. A new task can no
longer be silently filtered out.

## Adding a task

1. Put the task under `source_dataset` (`~/tb2` by default).
2. Build its prebaked harness image (see `eval/harbor/prebake/README.md`) and
   record the tag in the campaign `tasks[].image`.
3. Add the task entry with its public `source_image`; the runner checks that
   `task.toml` still names that source image.
4. Run the no-cost gate:

   ```bash
   python3 eval/harbor/bench5/run_matrix.py \
     --campaign eval/harbor/bench5/campaigns/bench5.yaml --preflight-only
   ```

   This checks Harbor 0.25.x, disk space, every image's toolchain with a real
   `docker run`, `docker image inspect`, the LiteCode binary/provenance, the
   proxy `/healthz`, and a tiny synthetic generation whose `usage=ok` row must
   land in the campaign's `egress_log`. It also prepares the dataset copy and
   records source-tree checksums.
5. Run the panel. At the end the runner invokes the collector's strict audit;
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

The image toolchain probe assumes the current supported base: Ubuntu 24.04,
root, `/root/.nvm`, `/root/.grok`, glibc and apt. Alpine/musl, non-root images
and images without bash need a separate toolchain adapter; the preflight will
reject them rather than letting them become missing trials. Three tasks are
still one trial each; pass@k and difficulty stratification remain a separate
evaluation decision.
The runner also writes `jobs_root/campaign.json`; a second campaign with a
different name cannot claim the same jobs root. Reusing an existing run id /
tag is refused unless `--allow-reuse-tag` is passed explicitly.
A filtered repair (`--task` / `--harness`) skips the whole-campaign strict
audit and prints a reminder; run the collector over the full campaign after
the repair so `selected.json` and the report reflect the complete matrix.