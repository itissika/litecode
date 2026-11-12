# LiteCode in Terminal-Bench (Harbor)

Three pieces, nothing else: a Linux build of the binary, a Harbor adapter that
installs it into a task container, and a one-row database that carries the
provider credential. LiteCode itself is unchanged — this directory only adds
the outside plumbing.

```
eval/
├── profile.toml        the scored surface: which tool groups a run may have
└── harbor/
    ├── Dockerfile          builder image: compiles the release binary on Linux
    ├── build.sh            stages the crate, builds, drops bin/litecode + seed db
    ├── litecode_agent.py   Harbor BaseInstalledAgent adapter
    ├── run_turn.sh         the one turn the adapter runs in the task container
    ├── test_litecode_agent.py  install-path tests (python -m unittest)
    └── bin/                build output (git-ignored)
        ├── litecode        the Linux binary
        ├── build-info.json commit, branch, dirty — written next to the binary
        └── litecode-seed.db  the app's own schema, with no credentials in it
```

## 1. Build

Run on the WSL side, where Docker lives. The builder starts from the locally
present `ubuntu:24.04`, installs rustup inside the container and compiles with
`--locked`; it also runs the built binary once to produce the database schema.

```bash
bash eval/harbor/build.sh
```

Nothing is compiled on the host, and nothing is compiled inside the task
container: task environments are the ones the dataset pins, so the adapter only
copies files.

## 2. Credentials

LiteCode reads its provider key from `provider_credentials` in its global
database — it does not read keys from the environment. So the adapter does not
mount anyone's settings: it takes the key from the environment Harbor was told
about, and writes exactly one row into a copy of the credential-free schema.
That database exists only inside the task container.

Keep the key in a private file outside the repo and hand it to the command that
starts Harbor:

```bash
chmod 600 ~/eval/api_key           # one line, the opencode-go key
LITECODE_API_KEY=$(cat ~/eval/api_key) harbor run ... --api-key-name LITECODE_API_KEY
```

`--api-key-name` is Harbor's own flag: it names the env var holding the key, and
the same name is what the adapter reads (`LITECODE_EVAL_KEY_ENV` overrides it).

The websearch key follows the same rule: it does not reach the container as an
environment variable or an argument, only as a settings row. The adapter writes
`EXA_API_KEY` to a 0600 file in its scratch directory and hands the CLI the
file (`config profile apply ... --websearch-key-file`); the key is never
printed, never in the applied document, and never on a command line. Without
that variable, websearch runs keyless. To set the key for a non-Harbor run,
write it from a file the same way:

```bash
printf '%s\n' "$(cat ~/eval/exa_key)" > /tmp/exa.key && chmod 600 /tmp/exa.key
litecode config profile apply eval/profile.toml --web off --websearch-key-file /tmp/exa.key
rm -f /tmp/exa.key
```

## 3. Which tools this run had

Installing a profile is a decision, so it is made at install time and recorded:

```bash
litecode config profile apply eval/profile.toml --web on|off
```

The adapter does not write bindings itself. It seeds the database copy, then
runs that same command (with HOME pointed at the copy) using the binary that is
about to be uploaded, so the CLI validates every tool id against the catalog it
ships with and applies it. The uploaded database is therefore the applied one.

Web tools follow the container, not the host: one probe of the task
environment (3 s, curl then wget then bash `/dev/tcp`) decides `--web`; any
failure — no route, no probe binary, a hung exec — means `off`, and never fails
the install. `EXA_API_KEY`, when Harbor has it in this process, is written into
the settings from a file (never a command line); without it websearch runs
keyless.

Installing with a profile is idempotent: apply twice, same rows.

## 4. Run one task

```bash
LITECODE_API_KEY=$(cat ~/eval/api_key) \
PYTHONPATH=$PWD \
harbor run -p ~/tb2 -i "<task-glob>" \
  -a eval.harbor.litecode_agent:LiteCode \
  -m opencode-go/deepseek-v4.1-flash \
  --api-key-name LITECODE_API_KEY \
  -n 1 -o ~/eval/jobs
```

Debug the install step alone first — it never enters the trial:

```bash
LITECODE_API_KEY=$(cat ~/eval/api_key) PYTHONPATH=$PWD harbor run ... --install-only
```

Notes:

* `-a` takes an import path, so `eval.harbor` must be importable: run from the
  repo root or set `PYTHONPATH`.
* `-m` decides the provider id (the part before `/`) and the model LiteCode
  runs: the adapter writes both into the seeded `default` agent.
* Network policy follows the task; this adapter adds none.

## 5. Artifacts

`/logs/agent` is mounted from the trial directory, so everything the run writes
there is kept:

| path | what it is |
| --- | --- |
| `agent/litecode.txt` | the turn's stdout+stderr, ending in the usage line |
| `agent/litecode/logs/litecode.log` | the CLI's JSON log: every request, tool call and step |
| `agent/litecode/sessions.db*` | the session database the UI would read |
| `agent/litecode/profile.json` | the applied profile: web resolved, tools, tiers, knobs, `network`, `websearch`, and the hashes |
| `agent/litecode/profile.applied.json` | the same document as the CLI wrote it; `profile.json` quotes its sha256 |
| `agent/litecode/build-info.json` | the binary's provenance, byte for byte, with the hashed copy linked from `profile.json` |
| `agent/trajectory.json` | the turn as ATIF-v1.8, one step per message and per model response, for `harbor view` |

The turn's numbers reach Harbor through the `AgentContext` that `result.json`
sums: `n_input_tokens` (cached tokens included), `n_cache_tokens`,
`n_output_tokens`, and `cost_usd`. `run()` fills it at the end of a successful
turn from that turn's `ExecResult` (`stdout`, then `stderr`) — `litecode.txt`
does not exist yet at that point, Harbor writes it afterwards. Harbor's
recovery hook (`populate_context_post_run`) is the same implementation for
cancelled or failed trials, and there the trial file is the text it reads. All
four come from the usage line described below — one parser, no second
accounting.
Cost is Harbor's LiteLLM price table when it can price the model; otherwise (and for any
zero it returns) the field stays `None`, and a run with no usage line leaves all
four unset rather than 0.

The usage line looks like

```
[litecode] completed in 4.8s — tokens prompt=11664 completion=7 cache_hit=0 cache_miss=11664
```

and the adapter parses it into `AgentContext`, so Harbor reports the same
numbers in its own summary.

## 6. The trajectory

`agent/trajectory.json` is written at the end of `run()`, from the turn's
`sessions.db` — table `transcript_items`, in `seq` order. That database is
pulled out of the container with `download_file` right after the turn: when
`run()` returns, Harbor has not copied `/logs/agent` to the host yet, so the
host directory is still empty and reading it in place would find nothing. The
`-wal`/`-shm` files come along when the container has them.

The file is ATIF-v1.8 and `harbor view` reads it from that path; no capability
flag is involved (`AgentCapabilities.atif` only gates `harbor traces` export).

One `item/user` row becomes a step with `source: "user"`. One model response
becomes a step with `source: "agent"`: its text in `message`, its reasoning in
`reasoning_content`, and the calls it made in `tool_calls` (`call_id` →
`tool_call_id`, the `arguments` JSON string parsed into an object). A
`item/tool_result` attaches to the step that made the call, as
`observation.results[]` with `source_call_id`. A `request/header` row marks
where a response begins, so it closes the step before it.

`final_metrics` quotes the same four numbers `run()` puts on the `AgentContext`
— one parse of the usage line, so the file and `result.json` cannot disagree —
plus `total_steps`. Nothing is invented. A database that cannot be read, a
session with no turn rows, or a document Harbor's ATIF model refuses all mean
no file, and one line on stderr says which; rows that cannot be read (a call
without an id or with arguments that are not JSON) are left out, also named on
stderr.


## 7. Which commit did this row run?

Every artifact carries its provenance. The tree that builds must be a git
checkout, and `build.sh` writes `bin/build-info.json` next to the binary:

```json
{
  "commit": "830cfc4a8b740b7265fc4fdb0928e97c3ed2a84b",
  "branch": "refactor/js-first-tool-contract",
  "dirty": false,
  "built_at": "2026-11-10T04:12:00Z",
  "rustc": "rustc 1.99.0 (b940084d7 2026-09-28)",
  "source_tree": "/home/sika/build/litecode-src"
}
```

Checking a row against the source you think it ran:

```bash
git -C ~/build/litecode-src rev-parse HEAD      # must equal build-info.json commit
git -C ~/build/litecode-src status --porcelain  # empty => "dirty": false is real
python3 -c 'import json;print(json.load(open("eval/harbor/bin/build-info.json"))["commit"])'
```

`dirty: true` means the working tree had uncommitted changes at build time: the
commit alone does not describe the binary, so such a row cannot be reproduced
from the repository alone. The adapter therefore **refuses to install** a dirty
build and names the `build-info.json` that said so; only
`LITECODE_EVAL_ALLOW_DIRTY=1` lets such a run go ahead, and it still records
`dirty: true` verbatim in the artifact. Rebuild from a clean checkout before
quoting a number.

A results table should quote exactly three fields from `build-info.json`:
`commit`, `branch`, `dirty`. `source_tree` is a local debugging aid — a
filesystem path that stops meaning anything on another machine — so it stays
out of the table.

The source tree is a **sparse checkout**: only the crate's inputs,
`sdk/generated`, `eval/harbor` and `eval/profile.toml` are materialised, so the
tree stays small while `HEAD`, branch and status still describe exactly what was
built. `eval/profile.toml` is there because the adapter hands that file to the
CLI at install time — without it, the install stops with "eval profile missing".
One consequence is worth saying out loud: **a clean `git status` here does not mean
the worktree is complete** — it means the materialised paths match the commit.
This tree is for building and packaging the eval binary, not a general
workspace; `git checkout -- .` will not restore files that were never
materialised.

## 8. When HEAD moves

The tree is a clone of the Windows checkout, so it has to be told when that
checkout advances. One command, either source:

```bash
# straight from the Windows checkout, as WSL sees it
eval/harbor/sync-source.sh /mnt/e/<drive>/litecode

# or from a bundle, for a machine that cannot read /mnt
git -C <windows> bundle create head.bundle refs/heads/refactor/js-first-tool-contract
eval/harbor/sync-source.sh head.bundle
```

The script fetches the branch the tree is on and fast-forwards to it; it prints
the new commit and warns if the working tree is dirty (which would make the next
`build-info.json` say `dirty: true`). Then rebuild, and the new
`build-info.json` carries the new commit:

```bash
bash eval/harbor/build.sh
```

## 9. Job configs for the other harnesses

`configs/` holds the four-layer job configs (bench / harness / model / job),
the same split and merge order workbuddy-bench documents in its
`configs/README.md`. `configs/resolve.py` deep-merges them into one Harbor job
YAML; the endpoint root comes from `EVAL_EGRESS_BASE_URL`, never from a file,
and a model-limit override is a hard error. See [configs/README.md](configs/README.md).

Harnesses configured so far: `pi`, `grok-build`, `opencode`, `codex` — each at
thinking high, on `deepseek/deepseek-flash`, through the egress proxy. Codex
speaks Responses through a native `config.toml`; the other three speak chat
completions.
