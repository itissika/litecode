"""The Harbor adapter's install path: two network answers, one artifact.

harbor is not a dependency of this repository, so the modules the adapter
imports are stubbed before it is imported. The rest is real: the CLI binary in
this checkout applies the repository profile to a seeded copy, and the artifact
is checked field by field. Without a built binary the tests skip.
"""

from __future__ import annotations

import asyncio
import contextlib
import hashlib
import io
import json
import os
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import types
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
PROFILE = REPO / "eval" / "profile.toml"


# ── harbor stubs ──────────────────────────────────────────────────────────


def _install_harbor_stubs() -> None:
    """Enough of harbor to import and drive the adapter without the package."""

    if "harbor" in sys.modules:
        return

    def module(name: str) -> types.ModuleType:
        created = types.ModuleType(name)
        created.__path__ = []  # a package, so the child imports below resolve
        sys.modules[name] = created
        return created

    def bind(module_object: types.ModuleType, name: str, value: object) -> None:
        setattr(module_object, name, value)

    harbor = module("harbor")
    agents = module("harbor.agents")
    installed = module("harbor.agents.installed")
    environments = module("harbor.environments")
    models = module("harbor.models")
    agent_models = module("harbor.models.agent")
    bind(harbor, "agents", agents)
    bind(harbor, "environments", environments)
    bind(harbor, "models", models)
    bind(agents, "installed", installed)
    bind(models, "agent", agent_models)

    class ModelConnectionSpec:
        pass

    class BaseEnvironment:
        pass

    class AgentContext:
        # None, not 0: an unmeasured run must not read as a free one.
        n_input_tokens: int | None = None
        n_output_tokens: int | None = None
        n_cache_tokens: int | None = None
        cost_usd: float | None = None

    def with_prompt_template(function):
        return function

    class BaseInstalledAgent:
        def __init__(self, model_name: str = "", logs_dir: Path = Path(".")) -> None:
            self.model_name = model_name
            self.logs_dir = Path(logs_dir)

        async def exec_as_root(self, environment, command: str):
            return await environment.exec(command, root=True)

        async def exec_as_agent(self, environment, command: str):
            return await environment.exec(command, root=False)

    auth = module("harbor.agents.agent_auth_resolution")
    base = module("harbor.agents.installed.base")
    env_base = module("harbor.environments.base")
    context = module("harbor.models.agent.context")
    bind(auth, "ModelConnectionSpec", ModelConnectionSpec)
    bind(base, "BaseInstalledAgent", BaseInstalledAgent)
    bind(base, "with_prompt_template", with_prompt_template)
    bind(env_base, "BaseEnvironment", BaseEnvironment)
    bind(context, "AgentContext", AgentContext)


_install_harbor_stubs()
sys.path.insert(0, str(Path(__file__).resolve().parent))

import litecode_agent  # noqa: E402

AgentContext = sys.modules["harbor.models.agent.context"].AgentContext


# ── doubles ───────────────────────────────────────────────────────────────


class Result:
    """What a Harbor exec returns, as far as the adapter reads it."""

    def __init__(
        self,
        status: int | None,
        stdout: str | None = None,
        stderr: str | None = None,
    ) -> None:
        if status is not None:
            self.exit_code = status
        self.stdout = stdout
        self.stderr = stderr


class FakeEnvironment:
    """Records uploads and execs; the egress probe gets the configured answer."""

    def __init__(
        self,
        probe: int | None | BaseException = 0,
        turn_stdout: str | None = None,
        turn_stderr: str | None = None,
        trial_db: Path | None = None,
        logs: Path | None = None,
    ) -> None:
        self.probe = probe
        self.turn_stdout = turn_stdout
        self.turn_stderr = turn_stderr
        self.trial_db = trial_db
        self.logs = logs
        self.uploads: list[tuple[Path, str]] = []
        self.commands: list[str] = []
        self.saved: dict[str, Path] = {}
        self._saved_dir = Path(tempfile.mkdtemp(prefix="litecode-upload-"))

    async def upload_file(self, local: Path, remote: str) -> None:
        self.uploads.append((Path(local), str(remote)))
        # keep a copy: install deletes its scratch HOME
        target = self._saved_dir / Path(remote).name
        shutil.copyfile(local, target)
        self.saved[str(remote)] = target

    async def exec(self, command: str, root: bool = False):
        self.commands.append(command)
        if "example.com" in command or "/dev/tcp/" in command:
            if isinstance(self.probe, BaseException):
                raise self.probe
            return Result(self.probe)
        if "litecode-run" in command:
            # run_turn.sh copies the turn's session database into the mounted
            # log directory; from then on it is a file on this side.
            if self.trial_db is not None and self.logs is not None:
                target = self.logs / "litecode"
                target.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(self.trial_db, target / "sessions.db")
            return Result(0, stdout=self.turn_stdout, stderr=self.turn_stderr)
        return Result(0)


def _find_binary() -> Path | None:
    override = os.environ.get("LITECODE_EVAL_BINARY")
    candidates = [Path(override)] if override else []
    candidates += [
        REPO / "eval" / "harbor" / "bin" / "litecode",
        REPO / "target" / "debug" / "litecode.exe",
        REPO / "target" / "debug" / "litecode",
        REPO / "target" / "release" / "litecode",
    ]
    return next((path for path in candidates if path.is_file()), None)


BINARY = _find_binary()


def _seeded_db(binary: Path, work: Path) -> Path:
    """A database with the app's schema and the seeded agents, and no key."""
    home = work / "seed-home"
    workspace = work / "seed-ws"
    workspace.mkdir(parents=True)
    subprocess.run(
        [str(binary), "--workspace", str(workspace), "list-sessions"],
        env=litecode_agent.LiteCode._isolated_env(home),
        capture_output=True,
        check=True,
    )
    return home / litecode_agent._DB_RELATIVE


def _session_store(binary: Path, work: Path) -> Path:
    """An empty sessions.db with the app's own schema, as a trial leaves it."""
    home = work / "store-home"
    workspace = work / "store-ws"
    workspace.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        [str(binary), "--workspace", str(workspace), "list-sessions"],
        env=litecode_agent.LiteCode._isolated_env(home),
        capture_output=True,
        check=True,
    )
    database = workspace / ".litecode" / "sessions.db"
    if not database.is_file():
        raise AssertionError(f"the CLI did not create {database}")
    return database


class Answered(litecode_agent.LiteCode):
    """The adapter with the probe injected: what a test asserts against."""

    answer = False

    async def _probe_network(self, environment) -> bool:  # noqa: ANN001 - test seam
        return self.answer


# ── tests ─────────────────────────────────────────────────────────────────


@unittest.skipIf(BINARY is None, "no LiteCode binary in this checkout")
class InstallTest(unittest.TestCase):
    def setUp(self) -> None:
        if BINARY is None:  # the class decorator already skipped; keeps typing honest
            self.skipTest("no LiteCode binary in this checkout")
        self.work = Path(tempfile.mkdtemp(prefix="litecode-test-"))
        self.addCleanup(shutil.rmtree, self.work, True)
        self.seed = _seeded_db(BINARY, self.work)
        self.session_store = _session_store(BINARY, self.work)
        self.build_info = self.work / "build-info.json"
        self.build_info.write_text(
            json.dumps({"commit": "c0ffee1", "branch": "eval-branch", "dirty": False}, indent=2)
            + "\n",
            encoding="utf-8",
        )
        self.logs = self.work / "logs"
        self.logs.mkdir()
        self.saved_env = dict(os.environ)
        os.environ.update(
            {
                "LITECODE_EVAL_BINARY": str(BINARY),
                "LITECODE_EVAL_SEED_DB": str(self.seed),
                "LITECODE_EVAL_PROFILE": str(PROFILE),
                "LITECODE_EVAL_BUILD_INFO": str(self.build_info),
                "LITECODE_API_KEY": "test-key",
                "EXA_API_KEY": "test-exa-key",
            }
        )
        self.addCleanup(self.restore_env)
        litecode_agent.LiteCode._pending_artifact = None

    def restore_env(self) -> None:
        os.environ.clear()
        os.environ.update(self.saved_env)

    def adapter(self, answer: bool) -> litecode_agent.LiteCode:
        agent = Answered(model_name="opencode-go/test-model", logs_dir=self.logs)
        agent.answer = answer
        return agent

    def artifacts(self) -> Path:
        return self.logs / litecode_agent._ARTIFACT_DIR

    def profile_record(self) -> dict:
        return json.loads((self.artifacts() / litecode_agent._PROFILE_FILE).read_text())

    def on_list(self) -> list[str]:
        import tomllib

        return sorted(tomllib.loads(PROFILE.read_text(encoding="utf-8"))["tools"]["on"])

    def test_network_on_binds_the_web_group_and_writes_the_artifact(self) -> None:
        environment = FakeEnvironment()
        asyncio.run(self.adapter(answer=True).install(environment))

        record = self.profile_record()
        applied = self.artifacts() / litecode_agent._APPLIED_FILE
        self.assertEqual(record["name"], "harbor")
        self.assertEqual(record["web"], "on")
        self.assertEqual(record["network"], "on")
        self.assertEqual(record["websearch"], "key")
        self.assertEqual(sorted(record["tools"]), sorted(self.on_list() + ["webfetch", "websearch"]))
        self.assertIn("webfetch", record["cards"])
        self.assertEqual(record["sha256"], hashlib.sha256(applied.read_bytes()).hexdigest())
        self.assertEqual(record["applied_file"], litecode_agent._APPLIED_FILE)

        # Build provenance: verbatim bytes, dirty flag untouched.
        copied = self.artifacts() / litecode_agent._BUILD_INFO_FILE
        self.assertEqual(copied.read_bytes(), self.build_info.read_bytes())
        provenance = json.loads(copied.read_text())
        self.assertEqual(provenance["commit"], "c0ffee1")
        self.assertIs(provenance["dirty"], False)
        self.assertEqual(
            record["build_info_sha256"], hashlib.sha256(copied.read_bytes()).hexdigest()
        )

        # The uploaded database is the applied one, not the seed.
        conn = sqlite3.connect(environment.saved["/tmp/litecode.db"])
        try:
            run = conn.execute(
                "SELECT enabled, preset FROM agent_tools "
                "WHERE agent_id = 'default' AND tool_id = 'run'"
            ).fetchone()
            enabled = conn.execute(
                "SELECT COUNT(*) FROM agent_tools WHERE agent_id = 'default' AND enabled = 1"
            ).fetchone()[0]
        finally:
            conn.close()
        self.assertEqual(run, (1, "ALL"))
        self.assertEqual(enabled, len(record["cards"]))

    def test_network_off_leaves_the_web_group_out(self) -> None:
        environment = FakeEnvironment()
        asyncio.run(self.adapter(answer=False).install(environment))

        record = self.profile_record()
        self.assertEqual(record["web"], "off")
        self.assertEqual(record["network"], "off")
        self.assertEqual(sorted(record["tools"]), self.on_list())
        self.assertNotIn("websearch", record["cards"])
        self.assertNotIn("websearch", record["tools"])

    def test_keyless_websearch_is_recorded(self) -> None:
        os.environ.pop("EXA_API_KEY", None)
        asyncio.run(self.adapter(answer=True).install(FakeEnvironment()))

        self.assertEqual(self.profile_record()["websearch"], "keyless")

    # The numbers are from a real trial: cache_hit + cache_miss == prompt.
    REAL_USAGE = "[litecode] completed in 41.2s — tokens prompt=57111 completion=3401 cache_hit=55424 cache_miss=1687"

    def one_user_turn(self) -> Path:
        return self.transcript(
            [
                (
                    "item/user",
                    {
                        "type": "message",
                        "role": "user",
                        "content": [{"type": "input_text", "text": "do the thing"}],
                    },
                )
            ]
        )

    def test_the_context_carries_this_turns_streams(self) -> None:
        environment = FakeEnvironment(
            turn_stdout=self.REAL_USAGE + "\n", trial_db=self.one_user_turn(), logs=self.logs
        )
        agent = self.adapter(answer=True)
        asyncio.run(agent.install(environment))
        installed = (
            litecode_agent._PROFILE_FILE,
            litecode_agent._APPLIED_FILE,
            litecode_agent._BUILD_INFO_FILE,
        )
        before = {name: (self.artifacts() / name).read_bytes() for name in installed}

        context = AgentContext()
        # The success path, with the real timing: at this point Harbor has not
        # written litecode.txt yet, so the numbers must come from the exec.
        asyncio.run(agent.run("do the thing", environment, context))
        self.assertFalse((self.logs / "litecode.txt").exists())

        # The fields Harbor sums into result.json, equal to the exec's line.
        self.assertEqual(context.n_input_tokens, 57111)
        self.assertEqual(context.n_cache_tokens, 55424)
        self.assertEqual(context.n_output_tokens, 3401)
        # Priced or unpriced, never a zero placeholder.
        self.assertTrue(context.cost_usd is None or context.cost_usd > 0)
        # The three install artifacts are not rewritten by a run.
        after = {name: (self.artifacts() / name).read_bytes() for name in installed}
        self.assertEqual(before, after)

    def test_the_usage_line_is_found_on_either_stream(self) -> None:
        environment = FakeEnvironment(
            turn_stderr=self.REAL_USAGE + "\n", trial_db=self.one_user_turn(), logs=self.logs
        )
        agent = self.adapter(answer=True)
        asyncio.run(agent.install(environment))
        context = AgentContext()
        asyncio.run(agent.run("do the thing", environment, context))
        self.assertEqual(
            (context.n_input_tokens, context.n_cache_tokens, context.n_output_tokens),
            (57111, 55424, 3401),
        )

    def test_the_recovery_hook_reads_the_trial_file(self) -> None:
        agent = self.adapter(answer=True)
        (self.logs / "litecode.txt").write_text(self.REAL_USAGE + "\n", encoding="utf-8")
        context = AgentContext()
        # Harbor calls the hook with no sources after recovering a failed trial.
        agent.populate_context_post_run(context)
        self.assertEqual(
            (context.n_input_tokens, context.n_cache_tokens, context.n_output_tokens),
            (57111, 55424, 3401),
        )

    def test_no_usage_line_leaves_the_context_unset(self) -> None:
        environment = FakeEnvironment(trial_db=self.one_user_turn(), logs=self.logs)
        agent = self.adapter(answer=True)
        asyncio.run(agent.install(environment))
        context = AgentContext()
        asyncio.run(agent.run("do the thing", environment, context))
        self.assertIsNone(context.n_input_tokens)
        self.assertIsNone(context.n_cache_tokens)
        self.assertIsNone(context.n_output_tokens)
        self.assertIsNone(context.cost_usd)

    def test_the_websearch_key_only_reaches_the_settings(self) -> None:
        key = "exa-secret-0123456789"
        os.environ["EXA_API_KEY"] = key
        environment = FakeEnvironment()
        asyncio.run(self.adapter(answer=True).install(environment))

        commands = "\n".join(environment.commands)
        self.assertNotIn(key, commands)
        for artifact in self.artifacts().iterdir():
            self.assertNotIn(key, artifact.read_text(encoding="utf-8", errors="replace"))
        conn = sqlite3.connect(environment.saved["/tmp/litecode.db"])
        try:
            row = conn.execute(
                "SELECT value FROM meta WHERE key = 'websearch.api_key'"
            ).fetchone()
        finally:
            conn.close()
        self.assertEqual(row[0], key)
        self.assertEqual(self.profile_record()["websearch"], "key")

    def test_a_dirty_build_is_refused_unless_allowed(self) -> None:
        self.build_info.write_text(
            json.dumps({"commit": "c0ffee1", "branch": "eval-branch", "dirty": True}),
            encoding="utf-8",
        )
        with self.assertRaises(RuntimeError) as refusal:
            asyncio.run(self.adapter(answer=True).install(FakeEnvironment()))
        message = str(refusal.exception)
        self.assertIn(str(self.build_info), message)
        self.assertIn("dirty", message)

        os.environ["LITECODE_EVAL_ALLOW_DIRTY"] = "1"
        asyncio.run(self.adapter(answer=True).install(FakeEnvironment()))
        provenance = json.loads(
            (self.artifacts() / litecode_agent._BUILD_INFO_FILE).read_text()
        )
        self.assertIs(provenance["dirty"], True)

    def test_a_failed_probe_is_off_not_a_failed_install(self) -> None:
        for probe in (RuntimeError("no route"), 1, None):
            with self.subTest(probe=probe):
                shutil.rmtree(self.artifacts(), ignore_errors=True)
                litecode_agent.LiteCode._pending_artifact = None
                environment = FakeEnvironment(probe=probe)
                asyncio.run(self.adapter(answer=False).install(environment))
                record = self.profile_record()
                self.assertEqual(record["network"], "off")
                self.assertEqual(record["web"], "off")
                # It went on to install: binary and database both landed.
                remotes = [remote for _local, remote in environment.uploads]
                self.assertIn("/tmp/litecode", remotes)
                self.assertIn("/tmp/litecode.db", remotes)
                self.assertTrue(any("litecode --help" in c for c in environment.commands))

    # ── the trajectory Harbor's viewer reads ─────────────────────────────

    SESSION = "01MTESTTRAJECTORY0000000A"

    _ITEM_TYPES = {
        "item/user": "message",
        "item/assistant": "message",
        "item/tool_call": "function_call",
        "item/tool_result": "function_call_output",
        "request/header": "request/header",
    }

    def transcript(self, rows: list[tuple[str, dict]]) -> Path:
        """The trial's sessions.db plus this turn's rows, as the CLI writes them."""
        copy = self.work / "trial.db"
        shutil.copyfile(self.session_store, copy)
        conn = sqlite3.connect(copy)
        try:
            for seq, (kind, body) in enumerate(rows):
                conn.execute(
                    "INSERT INTO transcript_items "
                    "(session_id, seq, item_type, kind, body, created_at, event_type, surface_op) "
                    "VALUES (?, ?, ?, ?, ?, ?, ?, 'append')",
                    (
                        self.SESSION,
                        seq,
                        self._ITEM_TYPES[kind],
                        kind,
                        json.dumps(body),
                        1_700_000_000_000 + seq,
                        kind,
                    ),
                )
            conn.commit()
            conn.execute("PRAGMA journal_mode=delete")
        finally:
            conn.close()
        return copy

    @staticmethod
    def message(text: str) -> dict:
        return {"type": "message", "role": "assistant", "content": [{"type": "text", "text": text}]}

    def test_the_run_writes_the_trajectory(self) -> None:
        rows: list[tuple[str, dict]] = [
            (
                "item/user",
                {
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "fix the password reset"}],
                },
            ),
            ("request/header", {"schema": 1, "step": 1}),
            (
                "item/assistant",
                {
                    "type": "reasoning",
                    "summary": [],
                    "content": [{"type": "reasoning_text", "text": "read the handler first"}],
                },
            ),
            (
                "item/tool_call",
                {
                    "type": "function_call",
                    "arguments": json.dumps({"path": "auth.py"}),
                    "call_id": "call_1",
                    "name": "read",
                },
            ),
            (
                "item/tool_result",
                {"type": "function_call_output", "call_id": "call_1", "output": "def reset(): ..."},
            ),
            ("request/header", {"schema": 1, "step": 2}),
            (
                "item/tool_call",
                {
                    "type": "function_call",
                    "arguments": json.dumps({"path": "auth.py", "old": "a", "new": "b"}),
                    "call_id": "call_2",
                    "name": "edit",
                },
            ),
            (
                "item/tool_result",
                {"type": "function_call_output", "call_id": "call_2", "output": "ok"},
            ),
            ("item/assistant", self.message("fixed the reset path")),
        ]
        database = self.transcript(rows)
        environment = FakeEnvironment(
            turn_stdout=self.REAL_USAGE + "\n", trial_db=database, logs=self.logs
        )
        agent = self.adapter(answer=True)
        asyncio.run(agent.install(environment))
        context = AgentContext()
        asyncio.run(agent.run("do the thing", environment, context))

        path = self.logs / litecode_agent._TRAJECTORY_FILE
        self.assertTrue(path.is_file(), "the trial directory must carry a trajectory")
        record = json.loads(path.read_text(encoding="utf-8"))

        self.assertEqual(record["schema_version"], "ATIF-v1.8")
        self.assertEqual(record["session_id"], self.SESSION)
        self.assertEqual(record["agent"]["name"], "litecode")
        self.assertEqual(record["agent"]["version"], "c0ffee1")

        steps = record["steps"]
        self.assertEqual([step["step_id"] for step in steps], [1, 2, 3])
        self.assertEqual([step["source"] for step in steps], ["user", "agent", "agent"])
        self.assertEqual(steps[0]["message"], "fix the password reset")

        # One tool call per item/tool_call row, in the order the CLI wrote them.
        calls = [call for step in steps for call in step.get("tool_calls", [])]
        self.assertEqual(len(calls), sum(1 for kind, _ in rows if kind == "item/tool_call"))
        self.assertEqual([call["function_name"] for call in calls], ["read", "edit"])
        self.assertEqual(calls[0]["arguments"], {"path": "auth.py"})
        self.assertEqual(steps[1]["reasoning_content"], "read the handler first")
        self.assertEqual(
            [result["source_call_id"] for result in steps[1]["observation"]["results"]],
            ["call_1"],
        )
        self.assertEqual(steps[2]["message"], "fixed the reset path")

        # final_metrics is the same accounting result.json gets, not a second one.
        metrics = record["final_metrics"]
        self.assertEqual(metrics["total_prompt_tokens"], context.n_input_tokens)
        self.assertEqual(metrics["total_completion_tokens"], context.n_output_tokens)
        self.assertEqual(metrics["total_cached_tokens"], context.n_cache_tokens)
        self.assertEqual(metrics["total_steps"], len(steps))
        if context.cost_usd is not None:
            self.assertEqual(metrics["total_cost_usd"], context.cost_usd)

        # Harbor's own model is the arbiter, when this machine has it.
        self.assertIsNone(litecode_agent._validate_trajectory(record))

    def test_a_session_without_rows_leaves_no_trajectory(self) -> None:
        database = self.transcript([])
        environment = FakeEnvironment(trial_db=database, logs=self.logs)
        agent = self.adapter(answer=True)
        asyncio.run(agent.install(environment))

        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr):
            asyncio.run(agent.run("do the thing", environment, AgentContext()))
        self.assertFalse((self.logs / litecode_agent._TRAJECTORY_FILE).exists())
        self.assertIn("no trajectory", stderr.getvalue())

    def test_an_unreadable_database_leaves_no_trajectory(self) -> None:
        broken = self.work / "broken.db"
        broken.write_bytes(b"this is not a database")
        environment = FakeEnvironment(trial_db=broken, logs=self.logs)
        agent = self.adapter(answer=True)
        asyncio.run(agent.install(environment))

        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr):
            asyncio.run(agent.run("do the thing", environment, AgentContext()))
        self.assertFalse((self.logs / litecode_agent._TRAJECTORY_FILE).exists())
        self.assertIn("unreadable", stderr.getvalue())



if __name__ == "__main__":
    unittest.main()
