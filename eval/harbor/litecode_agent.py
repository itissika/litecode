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
import json
import os
import re
import shlex
import shutil
import sqlite3
import subprocess
import sys
import tempfile
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
        await self.exec_as_agent(
            environment, command="litecode-run " + shlex.quote(instruction)
        )

    @override
    def populate_context_post_run(self, context: AgentContext) -> None:
        """Report the run's tokens from the usage line the CLI prints."""
        try:
            text = (self.logs_dir / "litecode.txt").read_text(encoding="utf-8", errors="replace")
        except OSError:
            return
        matches = _USAGE.findall(text)
        if not matches:
            return
        prompt, completion, cache_hit, _cache_miss = (int(v) for v in matches[-1])
        context.n_input_tokens = prompt
        context.n_cache_tokens = cache_hit
        context.n_output_tokens = completion
