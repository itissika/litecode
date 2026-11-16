"""Installed-agent adapters for a shared, prebuilt harness runtime.

Harbor's stock installed agents install their CLI on every trial. That is the
right default for a clean environment, but it makes a harness comparison pay
apt/nvm/npm costs inside every cell and turns a flaky registry into a failed
trial.  The eval runtime built by ``eval/harbor/runtime`` already carries the
exact binaries and is mounted read-only at /opt/harness; these subclasses
write a PATH shim, verify the binary, and then reuse the stock adapter's run
path, config writing and trajectory conversion.

The check is deliberately a check, not a silent no-op: an image without the
expected binary fails before the model is called, with the command and output
in the error.
"""

from __future__ import annotations

import shlex
from typing import override

from harbor.agents.installed.base import (
    BaseInstalledAgent,
    NonZeroAgentExitCodeError,
    with_prompt_template,
)
from harbor.agents.installed.codex import Codex
from harbor.agents.installed.grok_build import GrokBuild
from harbor.agents.installed.opencode import OpenCode
from harbor.agents.installed.pi import Pi
from harbor.environments.base import BaseEnvironment

_HARNESS_ROOT = "/opt/harness"


async def _install_runtime_path(
    agent: BaseInstalledAgent, environment: BaseEnvironment
) -> None:
    """Give the stock adapters a PATH shim instead of a real nvm install.

    The trial container is ephemeral, so writing ``~/.nvm/nvm.sh`` is safe.
    Codex guards its source with ``[ -s ... ]`` and Pi/OpenCode source it
    unconditionally; both then find the read-only runtime at /opt/harness.
    """
    command = (
        'mkdir -p "$HOME/.nvm" && '
        "printf '%s\\n' 'export PATH=\"/opt/harness/bin:$PATH\"' "
        '>> "$HOME/.nvm/nvm.sh"'
    )
    result = await agent.exec_as_agent(environment, command=command)
    if result.return_code != 0:
        detail = ((result.stdout or "") + (result.stderr or "")).strip()
        raise RuntimeError(
            "could not write the shared-runtime PATH shim; "
            f"`{command}` exited {result.return_code}: {detail or '<no output>'}"
        )


async def _require_binary(
    agent: BaseInstalledAgent,
    environment: BaseEnvironment,
    *,
    label: str,
    command: str,
) -> None:
    result = await agent.exec_as_agent(environment, command=command)
    if result.return_code != 0:
        detail = ((result.stdout or "") + (result.stderr or "")).strip()
        raise RuntimeError(
            f"prebaked {label} is not available in the shared runtime; "
            f"`{command}` exited {result.return_code}: {detail or '<no output>'}"
        )


class PrebakedPi(Pi):
    """pi from the shared runtime; skip the per-trial nvm + npm install."""

    @override
    async def install(self, environment: BaseEnvironment) -> None:
        await _install_runtime_path(self, environment)
        await _require_binary(
            self,
            environment,
            label="pi",
            command="[ -f ~/.nvm/nvm.sh ] && . ~/.nvm/nvm.sh; pi --version",
        )


class PrebakedGrokBuild(GrokBuild):
    """grok-build from the task image; skip the installer download."""

    @override
    async def install(self, environment: BaseEnvironment) -> None:
        await _require_binary(
            self,
            environment,
            label="grok-build",
            command=(
                'export PATH="$HOME/.grok/bin:/usr/local/bin:$PATH"; '
                "grok --version"
            ),
        )


class PrebakedOpenCode(OpenCode):
    """opencode from the shared runtime; skip apt + nvm + npm.

    It also replaces the stock ``| tee`` transport with a file redirect. In
    this Harbor/Docker combination the tee pipeline can close while opencode's
    final JSON event is still being written, producing ``EPIPE: broken pipe``
    and a non-zero harness exit *after* a correct answer. The file contains the
    same JSON-lines stream the stock adapter reads back for its trajectory; no
    model request or tool call changes.
    """

    @override
    async def install(self, environment: BaseEnvironment) -> None:
        await _install_runtime_path(self, environment)
        await _require_binary(
            self,
            environment,
            label="opencode",
            command=(
                "[ -f ~/.nvm/nvm.sh ] && . ~/.nvm/nvm.sh; "
                "opencode --version"
            ),
        )

    @override
    @with_prompt_template
    async def run(
        self,
        instruction: str,
        environment: BaseEnvironment,
        context: AgentContext,
    ) -> None:
        self._instruction = instruction
        escaped_instruction = shlex.quote(instruction)
        if not self.model_name:
            raise ValueError("Model name must be in the format provider/model_name")

        env = dict(self.model_connection.env)
        env["OPENCODE_FAKE_VCS"] = "git"
        env["XDG_DATA_HOME"] = (
            self.environment_logs_dir / "opencode" / "xdg-data"
        ).as_posix()
        env["XDG_STATE_HOME"] = (
            self.environment_logs_dir / "opencode" / "xdg-state"
        ).as_posix()

        skills_command = self._build_register_skills_command()
        if skills_command:
            await self.exec_as_agent(environment, command=skills_command, env=env)
        mcp_command = self._build_register_config_command()
        if mcp_command:
            await self.exec_as_agent(environment, command=mcp_command, env=env)

        cli_flags = self.build_cli_flags()
        cli_flags_arg = (cli_flags + " ") if cli_flags else ""
        resume_flag = "--continue " if self._resume else ""
        output_path = (
            self.environment_logs_dir / self._OUTPUT_FILENAME
        ).as_posix()

        await self.exec_as_agent(
            environment,
            command=(
                "[ -f ~/.nvm/nvm.sh ] && . ~/.nvm/nvm.sh; "
                f"opencode --model={shlex.quote(self._execution_model_name())} "
                f"run --format=json {resume_flag}{cli_flags_arg}--thinking "
                f"--dangerously-skip-permissions -- {escaped_instruction} "
                f"> {shlex.quote(output_path)} 2>&1; "
                "rc=$?; "
                'if [ "$rc" -ne 0 ]; then '
                # opencode 1.19.5 can emit an internal ``EPIPE: broken pipe``
                # after a completed turn. The JSON stream is the contract: if
                # it contains no error event and does contain a step_finish
                # with reason "stop", the turn completed. Anything else keeps
                # the original non-zero exit and its tail.
                f'  if grep -q \'"type":"error"\' {shlex.quote(output_path)}; then '
                f"    tail -c 4000 {shlex.quote(output_path)}; exit \"$rc\"; "
                "  fi; "
                f'  if grep -q \'"type":"step_finish".*"reason":"stop"\' {shlex.quote(output_path)}; then '
                '    exit 0; '
                "  fi; "
                f"  tail -c 4000 {shlex.quote(output_path)}; exit \"$rc\"; "
                "fi"
            ),
            env=env,
        )

        if messages := self._error_messages():
            raise NonZeroAgentExitCodeError(
                "OpenCode emitted error event(s): " + "; ".join(messages[:3])
            )


class PrebakedCodex(Codex):
    """codex from the shared runtime; skip the per-trial nvm + npm install."""

    @override
    async def install(self, environment: BaseEnvironment) -> None:
        await _install_runtime_path(self, environment)
        await _require_binary(
            self,
            environment,
            label="codex",
            command="[ -f ~/.nvm/nvm.sh ] && . ~/.nvm/nvm.sh; codex --version",
        )