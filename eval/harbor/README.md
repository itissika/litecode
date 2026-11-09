# LiteCode in Terminal-Bench (Harbor)

Three pieces, nothing else: a Linux build of the binary, a Harbor adapter that
installs it into a task container, and a one-row database that carries the
provider credential. LiteCode itself is unchanged — this directory only adds
the outside plumbing.

```
eval/harbor/
├── Dockerfile          builder image: compiles the release binary on Linux
├── build.sh            stages the crate, builds, drops bin/litecode + seed db
├── litecode_agent.py   Harbor BaseInstalledAgent adapter
├── run_turn.sh         the one turn the adapter runs in the task container
└── bin/                build output (git-ignored)
    ├── litecode        the Linux binary
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

## 3. Run one task

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

## 4. Artifacts

`/logs/agent` is mounted from the trial directory, so everything the run writes
there is kept:

| path | what it is |
| --- | --- |
| `agent/litecode.txt` | the turn's stdout+stderr, ending in the usage line |
| `agent/litecode/logs/litecode.log` | the CLI's JSON log: every request, tool call and step |
| `agent/litecode/sessions.db*` | the session database the UI would read |

The usage line looks like

```
[litecode] completed in 4.8s — tokens prompt=11664 completion=7 cache_hit=0 cache_miss=11664
```

and the adapter parses it into `AgentContext`, so Harbor reports the same
numbers in its own summary.

## 5. Which commit did this row run?

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
from the repository alone. Rebuild from a clean checkout before quoting a
number.

The source tree is a sparse checkout of the same commit as the Windows
workspace: only the crate's inputs, `sdk/generated` and `eval/harbor` are
materialised, so the working tree stays small while `HEAD`, branch and clean
status still describe exactly what was built.
