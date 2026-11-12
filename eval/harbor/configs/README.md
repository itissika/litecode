# Eval job configs

Four layers, same names and the same merge order as workbuddy-bench's
`configs/README.md`:

    bench -> harness -> model -> job        (later wins)

A layer only states what it owns. A job therefore never repeats a model or a
harness field it does not change, and the resolver refuses a job that would
override the model's own limits.

## Layout

| Path | Layer | Holds |
|---|---|---|
| `bench/_default.yaml` | 1 | trials, concurrency, jobs_dir, environment |
| `harnesses/<name>.yaml` | 2 | the CLI under test and its thinking level |
| `models/<name>.yaml` | 3 | provider/model name plus env var *names* for endpoint and key |
| `jobs/<name>.yaml` | 4 | which harnesses, on which model, over which dataset |
| `resolve.py` | — | deep-merges the layers into one Harbor job YAML |
| `test_resolve.py` | — | shapes, the thinking level, and the refusals |

## Render and run

    export EVAL_EGRESS_BASE_URL=http://<docker bridge>:8399   # see eval/proxy/README.md
    export LITELLM_LOCAL_MODEL_COST_MAP=True                  # no egress on the host
    python eval/harbor/configs/resolve.py \
      --job compare --dataset <dataset path> --run-id ds3-r1 > .tmp/job.yaml
    harbor run -c .tmp/job.yaml

Without `LITELLM_LOCAL_MODEL_COST_MAP=True` Harbor spends its first minutes
retrying a remote model-cost-map fetch that cannot succeed on a host without
egress.

The run id ends up inside every base_url (`/r/<run_id>-<harness>/v1`), so a
row in the proxy's JSONL names both the run and the harness without a join.

## What the resolver guarantees

* The endpoint root comes from `EVAL_EGRESS_BASE_URL` (or `--base-url`); the
  layer files hold no address. `localhost` and `127.0.0.1` are refused: inside
  a container they point at the container itself.
* The key is a variable *name* (`DEEPSEEK_API_KEY`); no value ever enters these
  files. The proxy replaces whatever arrives with the key from its own file.
* `context_window`, `max_output_tokens`, `max_tokens`,
  `max_completion_tokens` anywhere in the merged job is an error. The model
  decides its own limits; a bench number may not become the reason a run
  differs.
* Every harness must speak one of the model's declared protocols (`openai`
  for chat, `openai-responses` for Codex).
* A harness may set `base_url_suffix` (default `/v1`); Codex uses `/` because
  it appends `/responses` itself. Every other field stays the model's.

## Thinking level, per harness

| Harness | How | Value |
|---|---|---|
| pi | `kwargs.reasoning_effort` -> `pi --thinking` | high |
| grok-build | `kwargs.reasoning_effort` -> `grok --reasoning-effort` | high |
| opencode | config overlay `provider.deepseek.models.deepseek-flash.options.reasoningEffort` | high |
| codex | native `config.toml`: `model_reasoning_effort` | high |

DeepSeek's own default is already high, so a harness that ignores the knob
still runs at high; the note below only concerns whether the knob is honoured.

## Not verified here

* opencode's `reasoningEffort` overlay key: read from opencode's provider
  config docs, not yet seen in a run. If opencode drops it, the run is still
  high by DeepSeek's default.
* Nothing here has been run against the proxy yet; the architect smokes each
  harness in WSL first.
* Codex: its effective `config.toml` was built locally through Harbor 0.25.0's
  own adapter (`Codex._build_effective_config`), giving `model deepseek-flash`,
  provider `deepseek` with `wire_api = "responses"` and the per-run proxy
  `base_url`; no container has run it yet. `model_catalog_json` is omitted
  first, as agreed; add it only if codex refuses `deepseek-flash`.

## Tests

    cd eval/harbor/configs && python -m unittest test_resolve

Nine tests; the Harbor-shape test validates the rendered YAML through
`harbor.models.job.config.JobConfig` when Harbor is importable and skips
otherwise.
