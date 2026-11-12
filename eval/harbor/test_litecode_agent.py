"""The Harbor adapter's install path: two network answers, one artifact.

harbor is not a dependency of this repository, so the modules the adapter
imports are stubbed before it is imported. The rest is real: the CLI binary in
this checkout applies the repository profile to a seeded copy, and the artifact
is checked field by field. Without a built binary the tests skip.
"""

from __future__ import annotations

import asyncio
import hashlib
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

    def __init__(self, status: int | None) -> None:
        if status is not None:
            self.exit_code = status


class FakeEnvironment:
    """Records uploads and execs; the egress probe gets the configured answer."""

    def __init__(self, probe: int | None | BaseException = 0) -> None:
        self.probe = probe
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

    def test_the_context_carries_the_usage_line(self) -> None:
        environment = FakeEnvironment()
        agent = self.adapter(answer=True)
        asyncio.run(agent.install(environment))
        before = {path.name: path.read_bytes() for path in self.artifacts().iterdir()}

        # What run_turn.sh leaves behind after the turn (usage line only).
        (self.logs / "litecode.txt").write_text(
            "[litecode] completed in 4.8s — tokens prompt=11664 completion=7 "
            "cache_hit=3 cache_miss=11661\n",
            encoding="utf-8",
        )
        context = AgentContext()
        asyncio.run(agent.run("do the thing", environment, context))
        agent.populate_context_post_run(context)  # Harbor's own post-run hook

        # The fields Harbor sums into result.json, read from the usage line.
        self.assertEqual(context.n_input_tokens, 11664)
        self.assertEqual(context.n_cache_tokens, 3)
        self.assertEqual(context.n_output_tokens, 7)
        # Priced or unpriced, never a zero placeholder.
        self.assertTrue(context.cost_usd is None or context.cost_usd > 0)
        # The three install artifacts are not rewritten by a run.
        after = {path.name: path.read_bytes() for path in self.artifacts().iterdir()}
        self.assertEqual(before, after)

    def test_no_usage_line_leaves_the_context_unset(self) -> None:
        environment = FakeEnvironment()
        agent = self.adapter(answer=True)
        asyncio.run(agent.install(environment))
        context = AgentContext()
        asyncio.run(agent.run("do the thing", environment, context))
        agent.populate_context_post_run(context)
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


if __name__ == "__main__":
    unittest.main()
