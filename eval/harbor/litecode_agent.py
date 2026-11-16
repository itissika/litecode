"""Harbor installed-agent adapter for LiteCode.

Harbor runs installed agents inside a task container whose environment is
already built, so this adapter only does a few things there:

* install the Linux binary that build.sh produced (no Rust, no compiling),
* hand the run a global database holding the one provider credential it needs,
* apply the eval profile to that database with the CLI itself, and
* leave an artifact saying what was applied and what built it.

That database is the app's own schema (built by build.sh, no credentials in it)
plus a single provider_credentials row and the profile's bindings. Mounting a
user's whole settings database would leak every other provider key into the task
container, so it is not done.

The trial itself is one LiteCode turn started with --yes: no terminal exists
inside the container, and --yes is the only way such a run may act.
"""

from __future__ import annotations

import asyncio
import hashlib
import inspect
import json
import os
import re
import shlex
import shutil
import sqlite3
import subprocess
import sys
import tempfile
from collections.abc import Callable
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, override

from harbor.agents.agent_auth_resolution import ModelConnectionSpec
from harbor.agents.installed.base import BaseInstalledAgent, with_prompt_template
from harbor.environments.base import BaseEnvironment
from harbor.models.agent.context import AgentContext

# The line a headless LiteCode run leaves behind (see src/cli/cli.rs).
_USAGE = re.compile(r"tokens prompt=(\d+) completion=(\d+) cache_hit=(\d+) cache_miss=(\d+)")

# The key arrives as the env var Harbor names with --api-key-name; LiteCode
# itself does not read keys from the environment, so the adapter turns it into
# the database row the CLI does read.
_KEY_ENV = os.environ.get("LITECODE_EVAL_KEY_ENV", "LITECODE_API_KEY")

# $HOME-relative: the CLI resolves its global database from HOME and nowhere else.
_DB_RELATIVE = ".local/share/litecode/litecode.db"

# The scored surface: one file names every tool group a run may have. Applying
# it is the CLI's job — the adapter never writes bindings itself.
_PROFILE_RELATIVE = "../profile.toml"

# What /logs/agent keeps about this run: the applied profile, and the provenance
# of the binary that ran.
_ARTIFACT_DIR = "litecode"
_APPLIED_FILE = "profile.applied.json"
_PROFILE_FILE = "profile.json"
_BUILD_INFO_FILE = "build-info.json"

# ATIF is Harbor's trajectory format; the viewer reads this file and nothing
# else. The version is the one Harbor's model defaults to today.
_ATIF_VERSION = "ATIF-v1.8"
_TRAJECTORY_FILE = "trajectory.json"
_SESSION_DB = "sessions.db"

# The rows that carry the turn's conversation. Everything else in the
# database (reminders, request headers, turn markers) is bookkeeping.
_ITEM_KINDS = ("item/user", "item/assistant", "item/tool_call", "item/tool_result")

# The Exa key is the web channel's credential; without it websearch still runs,
# keyless and rate-limited. It reaches the run the same way the provider key
# does: a settings row in the database, never an argument or an artifact.
_EXA_ENV = "EXA_API_KEY"

# A build with uncommitted changes is not a version anyone can point at, so it
# is refused unless the caller says, explicitly, that this run may use one.
_ALLOW_DIRTY_ENV = "LITECODE_EVAL_ALLOW_DIRTY"

# One egress probe of the *container*: the web group is bound only if the task
# environment can actually reach the web. "timeout 3" bounds it inside the
# container; the host side bounds the exec too, in case the container hangs.
_PROBE_COMMAND = (
    "timeout 3 sh -c '"
    "curl -sSf --max-time 2 -o /dev/null https://example.com 2>/dev/null"
    " || wget -q -T 2 -O /dev/null https://example.com 2>/dev/null"
    ' || (command -v bash >/dev/null && bash -c "exec 3<>/dev/tcp/1.1.1.1/443" 2>/dev/null)'
    "'"
)
_PROBE_HOST_SECONDS = 8


def _cost_usd(model: str, prompt: int, completion: int, cached: int) -> float | None:
    """What the price table Harbor ships with (LiteLLM) says this turn cost.

    Unknown models, a missing table, a table that refuses these numbers, or a
    price that comes back as zero all mean "unpriced": the field stays None
    rather than guessing. `prompt` counts cached tokens too, as `n_input_tokens`
    does; when the table knows a cache-read rate, the cached part is priced at
    it.
    """
    try:
        from litellm import cost_per_token
    except Exception:
        return None
    arguments: dict[str, Any] = {
        "model": model,
        "prompt_tokens": prompt,
        "completion_tokens": completion,
    }
    try:
        parameters = inspect.signature(cost_per_token).parameters
    except (TypeError, ValueError):
        parameters = {}
    if "cache_read_input_tokens" in parameters:
        arguments["cache_read_input_tokens"] = cached
    try:
        prompt_cost, completion_cost = cost_per_token(**arguments)
    except Exception:
        return None
    if prompt_cost is None or completion_cost is None:
        return None
    total = float(prompt_cost) + float(completion_cost)
    # A table that returns zero for a priced model means "no price", not "free":
    # result.json must read a number or nothing, never 0.
    return total if total > 0 else None


def _usage(*texts: str | None) -> tuple[int, int, int, int] | None:
    """prompt, completion, cache_hit, cache_miss from the last usage line in `texts`.

    The line is the only accounting this adapter has; each caller hands in the
    text it actually holds — this turn's exec output, or the trial's log file
    when recovering a failed run — so the numbers cannot come from two places.
    """
    latest: tuple[str, str, str, str] | None = None
    for text in texts:
        if not text:
            continue
        matches = _USAGE.findall(text)
        if matches:
            latest = matches[-1]
    if latest is None:
        return None
    prompt, completion, cached, cache_miss = (int(value) for value in latest)
    return prompt, completion, cached, cache_miss


def _json_object(text: str | None) -> dict[str, Any] | None:
    """The row's body, when it is a JSON object."""
    if not text:
        return None
    try:
        body = json.loads(text)
    except ValueError:
        return None
    return body if isinstance(body, dict) else None


def _text_of(body: dict[str, Any] | None) -> str:
    """The text a message or reasoning body carries, in part order."""
    if body is None:
        return ""
    content = body.get("content")
    if not isinstance(content, list):
        return ""
    parts = [
        part["text"]
        for part in content
        if isinstance(part, dict) and isinstance(part.get("text"), str)
    ]
    return "\n".join(parts)


def _tool_call(body: dict[str, Any] | None) -> dict[str, Any] | None:
    """One ATIF tool call, or None when the row cannot name a complete one.

    A call without a name, an id or readable arguments is not invented back:
    the row is left out and the caller says so on stderr.
    """
    if body is None:
        return None
    name = body.get("name")
    call_id = body.get("call_id")
    if not isinstance(name, str) or not name:
        return None
    if not isinstance(call_id, str) or not call_id:
        return None
    raw = body.get("arguments")
    if isinstance(raw, dict):
        arguments = raw
    elif isinstance(raw, str):
        try:
            parsed = json.loads(raw)
        except ValueError:
            return None
        if not isinstance(parsed, dict):
            return None
        arguments = parsed
    else:
        return None
    return {"tool_call_id": call_id, "function_name": name, "arguments": arguments}


def _timestamp(milliseconds: Any) -> str | None:
    """ISO 8601 UTC for a row's created_at, when it is a real time."""
    if not isinstance(milliseconds, int) or milliseconds <= 0:
        return None
    return (
        datetime.fromtimestamp(milliseconds / 1000, tz=timezone.utc)
        .isoformat()
        .replace("+00:00", "Z")
    )


def _build_trajectory(
    rows: list[dict[str, Any]],
    *,
    session_id: str,
    agent_name: str,
    agent_version: str,
    model_name: str | None,
    usage: tuple[int, int, int, int] | None,
    cost: float | None,
    warn: Callable[[str], None],
) -> dict[str, Any] | None:
    """One ATIF document for the turn these rows describe, or None if empty.

    One assistant response is one step: a `request/header` row opens a response,
    so it closes the step before it, and the assistant/reasoning/tool_call rows
    after it belong together. Tool results attach to the step whose calls they
    answer, which is where ATIF puts an observation. Nothing is emitted that the
    database does not contain: a step without text, reasoning or calls is
    dropped, and a call or result that cannot be read is skipped out loud.
    """
    steps: list[dict[str, Any]] = []
    open_step: dict[str, Any] | None = None

    def start(created_at: Any) -> dict[str, Any]:
        return {
            "_text": [],
            "_reasoning": [],
            "_calls": [],
            "_results": [],
            "_timestamp": _timestamp(created_at),
        }

    def add(step: dict[str, Any]) -> None:
        step["step_id"] = len(steps) + 1
        steps.append(step)

    def close() -> None:
        nonlocal open_step
        if open_step is None:
            return
        text = "\n".join(open_step["_text"])
        reasoning = "\n".join(open_step["_reasoning"])
        calls = open_step["_calls"]
        results = open_step["_results"]
        timestamp = open_step["_timestamp"]
        open_step = None
        if not text and not reasoning and not calls:
            return
        step: dict[str, Any] = {"source": "agent", "message": text}
        if timestamp is not None:
            step["timestamp"] = timestamp
        if reasoning:
            step["reasoning_content"] = reasoning
        if calls:
            step["tool_calls"] = calls
            if results:
                step["observation"] = {"results": results}
        add(step)

    for row in rows:
        kind = row.get("kind")
        created_at = row.get("created_at")
        if kind == "request/header":
            # A model response begins here, so the previous one just ended.
            close()
            continue
        if kind not in _ITEM_KINDS:
            continue
        body = _json_object(row.get("body"))
        if kind == "item/user":
            close()
            text = _text_of(body)
            if not text:
                warn(f"a user row at seq {row.get('seq')} has no text")
                continue
            step = {"source": "user", "message": text}
            timestamp = _timestamp(created_at)
            if timestamp is not None:
                step["timestamp"] = timestamp
            add(step)
        elif kind == "item/assistant":
            if open_step is None:
                open_step = start(created_at)
            text = _text_of(body)
            if not text:
                continue
            if body is not None and body.get("type") == "reasoning":
                open_step["_reasoning"].append(text)
            else:
                open_step["_text"].append(text)
        elif kind == "item/tool_call":
            if open_step is None:
                open_step = start(created_at)
            call = _tool_call(body)
            if call is None:
                warn(f"a tool call at seq {row.get('seq')} cannot be read")
                continue
            open_step["_calls"].append(call)
        elif kind == "item/tool_result":
            if open_step is None:
                warn(f"a tool result at seq {row.get('seq')} answers no call")
                continue
            call_id = body.get("call_id") if body is not None else None
            if not isinstance(call_id, str) or call_id not in {
                call["tool_call_id"] for call in open_step["_calls"]
            }:
                warn(f"a tool result at seq {row.get('seq')} answers no call in its step")
                continue
            result: dict[str, Any] = {"source_call_id": call_id}
            output = body.get("output") if body is not None else None
            if isinstance(output, str):
                result["content"] = output
            open_step["_results"].append(result)
    close()

    if not steps:
        return None

    record: dict[str, Any] = {
        "schema_version": _ATIF_VERSION,
        "agent": {
            "name": agent_name,
            "version": agent_version,
            "model_name": model_name,
        },
        "steps": steps,
    }
    if session_id:
        record["session_id"] = session_id
    if usage is not None:
        prompt, completion, cached, _cache_miss = usage
        metrics: dict[str, Any] = {
            "total_prompt_tokens": prompt,
            "total_completion_tokens": completion,
            "total_cached_tokens": cached,
            "total_steps": len(steps),
        }
        if cost is not None:
            metrics["total_cost_usd"] = cost
        record["final_metrics"] = metrics
    return record


def _read_transcript(database: Path) -> tuple[str, list[dict[str, Any]]]:
    """The session with the newest activity and its rows, in seq order.

    Copied aside first: the trial's copy may still have a WAL beside it, and a
    read must not leave journal files in an artifact.
    """
    with tempfile.TemporaryDirectory(prefix="litecode-trajectory-") as scratch:
        copy = Path(scratch) / database.name
        shutil.copyfile(database, copy)
        for suffix in ("-wal", "-shm"):
            journal = database.with_name(database.name + suffix)
            if journal.is_file():
                shutil.copyfile(journal, copy.with_name(copy.name + suffix))
        conn = sqlite3.connect(copy)
        try:
            chosen = conn.execute(
                "SELECT session_id FROM transcript_items "
                "GROUP BY session_id ORDER BY MAX(created_at) DESC LIMIT 1"
            ).fetchone()
            if chosen is None:
                return "", []
            session_id = str(chosen[0])
            rows = [
                {
                    "seq": seq,
                    "kind": kind,
                    "body": body,
                    "created_at": created_at,
                }
                for seq, kind, body, created_at in conn.execute(
                    "SELECT seq, kind, body, created_at FROM transcript_items "
                    "WHERE session_id = ? ORDER BY seq",
                    (session_id,),
                )
            ]
        finally:
            conn.close()
    return session_id, rows


def _validate_trajectory(record: dict[str, Any]) -> str | None:
    """Harbor's own ATIF model as the arbiter.

    None means the model accepted it, or that this Harbor ships no model to
    check against (an import that fails is not a verdict). Anything else is the
    reason it refused.
    """
    try:
        from harbor.models.trajectories.trajectory import Trajectory
    except Exception:
        return None
    try:
        Trajectory.model_validate(record)
    except Exception as error:
        return str(error)
    return None


def _exit_status(result: Any) -> int | None:
    """The exec status of a Harbor result, whatever field it carries it in."""
    for name in ("exit_code", "return_code", "code"):
        value = getattr(result, name, None)
        if isinstance(value, int):
            return value
    return None


class LiteCode(BaseInstalledAgent):
    """LiteCode as an installed agent: one scripted turn per task."""

    # LiteCode resolves its endpoint from the catalog it ships with, so Harbor
    # has nothing to route: it only names the environment variable holding the
    # key, and the adapter turns that into the database row the CLI reads.
    MODEL_CONNECTION = ModelConnectionSpec()

    # Set when the logs mount cannot be written during install (some Harbor
    # versions only mount it for the trial); run() writes it before the turn.
    _pending_artifact: tuple[dict[str, Any], bytes, bytes, bool, str] | None = None

    # The commit the binary came from, as recorded by build.sh; the trajectory
    # names it so a step can be traced back to the build that made it.
    _build_version = "unknown"

    @staticmethod
    @override
    def name() -> str:
        return "litecode"

    # ── host inputs ───────────────────────────────────────────────────────

    @staticmethod
    def _path(env_name: str, default: str) -> Path:
        override = os.environ.get(env_name)
        return Path(override).expanduser() if override else Path(__file__).resolve().parent / default

    def _model_ref(self) -> str:
        if not self.model_name:
            raise RuntimeError(
                "LiteCode needs a provider/model model name: pass -m "
                "opencode-go/deepseek-v4.1-flash"
            )
        return self.model_name

    @staticmethod
    def _isolated_env(home: Path) -> dict[str, str]:
        """An environment whose only global database is the copy under home.

        Linux reads $HOME; Windows reads %LOCALAPPDATA%. Pointing the latter at
        home/.local/share makes both resolve to the same file, which is the one
        that gets uploaded.
        """
        env = os.environ.copy()
        env["HOME"] = str(home)
        env["USERPROFILE"] = str(home)
        env["LOCALAPPDATA"] = str(home / ".local" / "share")
        env.pop("XDG_DATA_HOME", None)
        return env

    def _credential_home(self, seed: Path, api_key: str) -> Path:
        """Copy the credential-free schema into a private HOME and add one row."""
        model_ref = self._model_ref()
        provider_id = model_ref.split("/", 1)[0]

        home = Path(tempfile.mkdtemp(prefix="litecode-home-"))
        path = home / _DB_RELATIVE
        path.parent.mkdir(parents=True)
        try:
            shutil.copyfile(seed, path)
            # build.sh ships the app's WAL alongside the database; copying it too
            # leaves one file to hand to the container once it is folded in.
            wal = seed.with_name(seed.name + "-wal")
            if wal.is_file():
                shutil.copyfile(wal, path.with_name(path.name + "-wal"))
            conn = sqlite3.connect(path)
            try:
                conn.execute(
                    "INSERT OR REPLACE INTO provider_credentials (provider_id, api_key) "
                    "VALUES (?, ?)",
                    (provider_id, api_key),
                )
                # The seeded agent has no model; this run's model is the truth.
                conn.execute(
                    "UPDATE agents SET model_ref = ? WHERE role = 'primary' AND id = 'default'",
                    (model_ref,),
                )
                conn.commit()
                conn.execute("PRAGMA journal_mode=delete")
            finally:
                conn.close()
            self._fold_journal(path)
            os.chmod(path, 0o600)
        except BaseException:
            shutil.rmtree(home, ignore_errors=True)
            raise
        return home

    @staticmethod
    def _fold_journal(database: Path) -> None:
        """Leave one self-contained file behind: no WAL, no shm."""
        if database.is_file():
            conn = sqlite3.connect(database)
            try:
                conn.execute("PRAGMA wal_checkpoint(TRUNCATE)")
                conn.execute("PRAGMA journal_mode=delete")
            finally:
                conn.close()
        for suffix in ("-wal", "-shm"):
            database.with_name(database.name + suffix).unlink(missing_ok=True)

    def _apply_profile(
        self, home: Path, binary: Path, profile: Path, web: str, websearch_key: str | None
    ) -> tuple[dict[str, Any], bytes]:
        """Let the CLI apply the profile to that copy.

        The row layout stays in Rust: this only passes the file, the resolved
        web answer, where to write the applied document, and — as a file, never
        as a value — the websearch key that belongs in the settings.
        """
        applied_path = home / "applied.json"
        workspace = home / "ws"
        workspace.mkdir()
        arguments = [
            str(binary),
            "--workspace",
            str(workspace),
            "config",
            "profile",
            "apply",
            str(profile),
            "--web",
            web,
            "--write-applied",
            str(applied_path),
        ]
        if websearch_key is not None:
            # 0600 next to the database copy: it leaves with the scratch HOME,
            # and the key itself never becomes an argument.
            key_file = home / "websearch.key"
            key_file.write_text(websearch_key + "\n", encoding="utf-8")
            os.chmod(key_file, 0o600)
            arguments += ["--websearch-key-file", str(key_file)]
        proc = subprocess.run(
            arguments,
            env=self._isolated_env(home),
            capture_output=True,
            text=True,
        )
        if proc.returncode != 0:
            detail = (proc.stderr or proc.stdout).strip()
            raise RuntimeError(f"applying {profile} failed ({proc.returncode}): {detail}")
        raw = applied_path.read_bytes()
        # The CLI opens the database in WAL mode; upload one file.
        self._fold_journal(home / _DB_RELATIVE)
        return json.loads(raw), raw

    async def _probe_network(self, environment: BaseEnvironment) -> bool:
        """Whether this container can reach the web.

        Anything other than a clean success — a failed probe, a missing curl, a
        hung exec, an exec result this version cannot read — means "off". A
        probe must never fail an install.
        """
        try:
            result = await asyncio.wait_for(
                self.exec_as_agent(environment, command=_PROBE_COMMAND),
                timeout=_PROBE_HOST_SECONDS,
            )
        except Exception:
            return False
        return _exit_status(result) == 0

    def _write_artifact(
        self,
        applied: dict[str, Any],
        applied_bytes: bytes,
        build_info: bytes,
        network: bool,
        websearch: str,
    ) -> None:
        """Leave what this run was: the applied profile and the build it ran."""
        directory = Path(self.logs_dir) / _ARTIFACT_DIR
        directory.mkdir(parents=True, exist_ok=True)
        (directory / _APPLIED_FILE).write_bytes(applied_bytes)
        (directory / _BUILD_INFO_FILE).write_bytes(build_info)
        record = {
            **applied,
            "sha256": hashlib.sha256(applied_bytes).hexdigest(),
            "applied_file": _APPLIED_FILE,
            "build_info_file": _BUILD_INFO_FILE,
            "build_info_sha256": hashlib.sha256(build_info).hexdigest(),
            "network": "on" if network else "off",
            "websearch": websearch,
        }
        (directory / _PROFILE_FILE).write_text(
            json.dumps(record, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )

    # ── what the turn used ───────────────────────────────────────────────

    def _trial_log(self) -> str | None:
        """The trial's `litecode.txt`, when it has already been written."""
        try:
            return (self.logs_dir / "litecode.txt").read_text(
                encoding="utf-8", errors="replace"
            )
        except OSError:
            return None

    async def _download_transcript(self, environment: BaseEnvironment) -> Path | None:
        """The turn's `sessions.db`, pulled out of the container.

        When run() returns, Harbor has not copied `/logs/agent` to the host yet —
        the host directory is still empty — so the database is fetched by name
        instead of read in place. The WAL files travel with it when the copy
        has them. A database that is not there is None, and the caller says so
        once: a run with no transcript reports no trajectory, never a guess.
        """
        scratch = Path(tempfile.mkdtemp(prefix="litecode-transcript-"))
        target = scratch / _SESSION_DB
        remote = f"/logs/agent/{_ARTIFACT_DIR}/{_SESSION_DB}"

        async def present(path: str) -> bool:
            # Docker's failed `compose cp` is noisy. Ask first when the
            # environment can answer; older test doubles cannot.
            checker = getattr(environment, "is_file", None)
            if checker is None:
                return True
            try:
                return bool(await checker(path))
            except Exception:
                return True

        if not await present(remote):
            shutil.rmtree(scratch, ignore_errors=True)
            return None
        try:
            await environment.download_file(remote, target)
        except Exception:
            shutil.rmtree(scratch, ignore_errors=True)
            return None
        if not target.is_file():
            shutil.rmtree(scratch, ignore_errors=True)
            return None
        for suffix in ("-wal", "-shm"):
            if not await present(remote + suffix):
                continue
            try:
                await environment.download_file(
                    remote + suffix, target.with_name(target.name + suffix)
                )
            except Exception:
                # A copy without its journal is still a database.
                continue
        return target

    def _write_trajectory(
        self,
        usage: tuple[int, int, int, int] | None,
        cost: float | None,
        database: Path | None,
    ) -> None:
        """Leave `trajectory.json`: what this turn actually did, step by step.

        The rows come from the database `run()` downloaded, the same one the
        trial keeps under /logs/agent. A database that could not be downloaded
        or read, a session with no turn rows, or a document Harbor's model
        refuses all mean no file: a trajectory is a record, and a made-up one
        is worse than a missing one. Each of those says so on stderr.
        """
        if database is None:
            print(
                "litecode adapter: no trajectory: the turn's sessions.db could not "
                "be downloaded from the container",
                file=sys.stderr,
            )
            return
        try:
            session_id, rows = _read_transcript(database)
        except (OSError, sqlite3.Error) as error:
            print(
                f"litecode adapter: no trajectory: {database} unreadable ({error})",
                file=sys.stderr,
            )
            return
        announced: set[str] = set()

        def warn(message: str) -> None:
            # One line, however often the same row shape repeats.
            if message in announced:
                return
            announced.add(message)
            print(f"litecode adapter: trajectory left a row out: {message}", file=sys.stderr)

        record = _build_trajectory(
            rows,
            session_id=session_id,
            agent_name=self.name(),
            agent_version=self._build_version,
            model_name=self.model_name or None,
            usage=usage,
            cost=cost,
            warn=warn,
        )
        if record is None:
            print(
                f"litecode adapter: no trajectory: {database} has no turn rows",
                file=sys.stderr,
            )
            return
        complaint = _validate_trajectory(record)
        if complaint is not None:
            print(
                f"litecode adapter: no trajectory: Harbor's ATIF model refused it "
                f"({complaint})",
                file=sys.stderr,
            )
            return
        target = Path(self.logs_dir) / _TRAJECTORY_FILE
        target.write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")

    # ── install / run ────────────────────────────────────────────────────

    @override
    async def install(self, environment: BaseEnvironment) -> None:
        binary = self._path("LITECODE_EVAL_BINARY", "bin/litecode")
        seed = self._path("LITECODE_EVAL_SEED_DB", "bin/litecode-seed.db")
        profile = self._path("LITECODE_EVAL_PROFILE", _PROFILE_RELATIVE)
        build_info = self._path("LITECODE_EVAL_BUILD_INFO", "bin/build-info.json")
        api_key = os.environ.get(_KEY_ENV, "").strip()
        if not binary.is_file():
            raise RuntimeError(
                f"prebuilt LiteCode binary missing: {binary} — run eval/harbor/build.sh"
            )
        if not seed.is_file():
            raise RuntimeError(
                f"credential-free database missing: {seed} — run eval/harbor/build.sh"
            )
        if not profile.is_file():
            raise RuntimeError(
                f"eval profile missing: {profile} — run eval/harbor/build.sh "
                "or set LITECODE_EVAL_PROFILE"
            )
        if not build_info.is_file():
            raise RuntimeError(
                f"build provenance missing: {build_info} — run eval/harbor/build.sh"
            )
        if not api_key:
            raise RuntimeError(
                f"{_KEY_ENV} is empty: name the key env with --api-key-name {_KEY_ENV}"
            )

        # Copied byte for byte: a row has to be quotable without rewriting it, so
        # dirty: true stays visible instead of being tidied away.
        provenance = build_info.read_bytes()
        try:
            provenance_record = json.loads(provenance)
        except ValueError as error:
            raise RuntimeError(f"{build_info} is not JSON: {error}") from error
        if not provenance_record.get("commit"):
            raise RuntimeError(f"{build_info} does not name a commit")
        self._build_version = str(provenance_record["commit"])[:12]
        dirty = bool(provenance_record.get("dirty"))
        allowed_to_be_dirty = os.environ.get(_ALLOW_DIRTY_ENV, "").strip() == "1"
        if dirty and not allowed_to_be_dirty:
            raise RuntimeError(
                f"build provenance says the working tree was dirty: {build_info} "
                '(has "dirty": true) — rebuild from a clean checkout, or set '
                f"{_ALLOW_DIRTY_ENV}=1 to run it anyway"
            )

        # The web group follows the container's network, not the host's.
        network = await self._probe_network(environment)
        web = "on" if network else "off"
        websearch_key = os.environ.get(_EXA_ENV, "").strip() or None
        websearch = "key" if websearch_key else "keyless"

        home = self._credential_home(seed, api_key)
        try:
            applied, applied_bytes = await asyncio.to_thread(
                self._apply_profile, home, binary, profile, web, websearch_key
            )
            try:
                self._write_artifact(applied, applied_bytes, provenance, network, websearch)
                self._pending_artifact = None
            except OSError as error:
                # Some Harbor versions only mount /logs/agent for the trial.
                print(f"litecode adapter: artifact not written yet: {error}", file=sys.stderr)
                self._pending_artifact = (
                    applied,
                    applied_bytes,
                    provenance,
                    network,
                    websearch,
                )

            await environment.upload_file(binary, "/tmp/litecode")
            await self.exec_as_root(
                environment,
                command="install -m 0755 /tmp/litecode /usr/local/bin/litecode && rm -f /tmp/litecode",
            )

            # The run user's HOME is the only place the CLI looks for it, and
            # the upload lands as root, so it is handed over inside the container
            # and then narrowed to 0600 for that user.
            await environment.upload_file(home / _DB_RELATIVE, "/tmp/litecode.db")
            await self.exec_as_root(environment, command="chmod 0644 /tmp/litecode.db")
            await self.exec_as_agent(
                environment,
                command=(
                    "set -euo pipefail; "
                    f'install -d -m 700 "$HOME/{Path(_DB_RELATIVE).parent.as_posix()}"; '
                    f'install -m 600 /tmp/litecode.db "$HOME/{_DB_RELATIVE}"; '
                    "rm -f /tmp/litecode.db"
                ),
            )
        finally:
            shutil.rmtree(home, ignore_errors=True)

        await self.exec_as_agent(environment, command="litecode --help >/dev/null")

    @override
    @with_prompt_template
    async def run(
        self,
        instruction: str,
        environment: BaseEnvironment,
        context: AgentContext,
    ) -> None:
        pending = self._pending_artifact
        if pending is not None:
            self._pending_artifact = None
            try:
                self._write_artifact(*pending)
            except OSError as error:
                print(f"litecode adapter: artifact still not writable: {error}", file=sys.stderr)

        await environment.upload_file(
            Path(__file__).resolve().parent / "run_turn.sh", "/tmp/litecode-run.sh"
        )
        await self.exec_as_root(
            environment,
            command=(
                "install -m 0755 /tmp/litecode-run.sh /usr/local/bin/litecode-run "
                "&& rm -f /tmp/litecode-run.sh"
            ),
        )
        # No key on this command line: the turn reads it from the settings the
        # install wrote, the same way it reads the provider key.
        result = await self.exec_as_agent(
            environment, command="litecode-run " + shlex.quote(instruction)
        )
        # `litecode.txt` is written by Harbor after this returns, so at this point
        # the usage line exists only in this exec's streams. Both are read: the
        # CLI prints its summary to whichever one it was given.
        streams = (getattr(result, "stdout", None), getattr(result, "stderr", None))
        usage = _usage(*streams)
        # One parse, two readers: result.json and the trajectory quote the same
        # numbers because both come from this tuple.
        self._report(context, usage)
        # Harbor copies /logs/agent to the host only after this returns, so the
        # transcript is fetched from the container while it is still there.
        database = await self._download_transcript(environment)
        try:
            self._write_trajectory(usage, context.cost_usd, database)
        finally:
            if database is not None:
                shutil.rmtree(database.parent, ignore_errors=True)

    def _report(self, context: AgentContext, usage: tuple[int, int, int, int] | None) -> None:
        """Put one usage line on the context Harbor sums into result.json."""
        if usage is None:
            return
        prompt, completion, cached, _cache_miss = usage
        context.n_input_tokens = prompt
        context.n_cache_tokens = cached
        context.n_output_tokens = completion
        context.cost_usd = _cost_usd(self.model_name, prompt, completion, cached)

    @override
    def populate_context_post_run(self, context: AgentContext, *sources: str | None) -> None:
        """Report the run's usage in the fields Harbor sums into result.json.

        Two entries share this one implementation. run() passes the exec streams
        it just received. Harbor calls the hook with no sources when recovering
        outputs from a cancelled or failed trial; then the trial's log file is
        the only text left. This is the whole accounting: Harbor reads
        n_input_tokens (cached tokens included), n_cache_tokens,
        n_output_tokens and cost_usd from the context. No usage line means the
        fields stay unset (None), never 0: an unmeasured run must not look like
        a free one.
        """
        self._report(context, _usage(*(sources or (self._trial_log(),))))
