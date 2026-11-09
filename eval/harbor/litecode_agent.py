"""Harbor installed-agent adapter for LiteCode.

Harbor runs installed agents inside a task container whose environment is
already built, so this adapter only does two things there:

* install the Linux binary that `build.sh` produced (no Rust, no compiling),
* hand the run a global database holding the one provider credential it needs.

That database is the app's own schema (built by `build.sh`, no credentials in
it) plus a single `provider_credentials` row. Mounting a user's whole settings
database would leak every other provider key into the task container, so it is
not done.

The trial itself is one LiteCode turn started with `--yes`: no terminal exists
inside the container, and `--yes` is the only way such a run may act.
"""

from __future__ import annotations

import os
import re
import shlex
import shutil
import sqlite3
import tempfile
from pathlib import Path
from typing import override

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


class LiteCode(BaseInstalledAgent):
    """LiteCode as an installed agent: one scripted turn per task."""

    # LiteCode resolves its endpoint from the catalog it ships with, so Harbor
    # has nothing to route: it only names the environment variable holding the
    # key, and the adapter turns that into the database row the CLI reads.
    MODEL_CONNECTION = ModelConnectionSpec()

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

    def _credential_db(self, seed: Path, api_key: str) -> Path:
        """Copy the credential-free schema and add exactly one provider row."""
        model_ref = self._model_ref()
        provider_id = model_ref.split("/", 1)[0]

        work = Path(tempfile.mkdtemp(prefix="litecode-home-"))
        path = work / "litecode.db"
        try:
            shutil.copyfile(seed, path)
            # build.sh ships the app's WAL alongside the database; folding it in
            # leaves one file to hand to the container.
            wal = seed.with_name(seed.name + "-wal")
            if wal.is_file():
                shutil.copyfile(wal, work / "litecode.db-wal")
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
            os.chmod(path, 0o600)
        except BaseException:
            shutil.rmtree(work, ignore_errors=True)
            raise
        return path

    # ── install / run ────────────────────────────────────────────────────

    @override
    async def install(self, environment: BaseEnvironment) -> None:
        binary = self._path("LITECODE_EVAL_BINARY", "bin/litecode")
        seed = self._path("LITECODE_EVAL_SEED_DB", "bin/litecode-seed.db")
        api_key = os.environ.get(_KEY_ENV, "").strip()
        if not binary.is_file():
            raise RuntimeError(f"prebuilt LiteCode binary missing: {binary} — run eval/harbor/build.sh")
        if not seed.is_file():
            raise RuntimeError(f"credential-free database missing: {seed} — run eval/harbor/build.sh")
        if not api_key:
            raise RuntimeError(f"{_KEY_ENV} is empty: name the key env with --api-key-name {_KEY_ENV}")

        database = self._credential_db(seed, api_key)
        try:
            await environment.upload_file(binary, "/tmp/litecode")
            await self.exec_as_root(
                environment,
                command="install -m 0755 /tmp/litecode /usr/local/bin/litecode && rm -f /tmp/litecode",
            )

            # The run user's HOME is the only place the CLI looks for it, and
            # the upload lands as root, so it is handed over inside the container
            # and then narrowed to 0600 for that user.
            await environment.upload_file(database, "/tmp/litecode.db")
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
            shutil.rmtree(database.parent, ignore_errors=True)

        await self.exec_as_agent(environment, command="litecode --help >/dev/null")

    @override
    @with_prompt_template
    async def run(
        self,
        instruction: str,
        environment: BaseEnvironment,
        context: AgentContext,
    ) -> None:
        await environment.upload_file(
            Path(__file__).resolve().parent / "run_turn.sh", "/tmp/litecode-run.sh"
        )
        await self.exec_as_root(
            environment,
            command="install -m 0755 /tmp/litecode-run.sh /usr/local/bin/litecode-run && rm -f /tmp/litecode-run.sh",
        )
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
