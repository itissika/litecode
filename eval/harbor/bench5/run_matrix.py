#!/usr/bin/env python3
"""Run one campaign: one Harbor job per task, one agent per harness per job.

Why one job per task instead of one job for the whole campaign:

* the egress proxy names a request by the base_url's run id;
* Harbor gives every agent in a job one fixed base_url, so a single job would
  merge tasks into one bill per harness;
* ``<campaign>-<task>-<attempt>`` gives every (task, harness) cell its own run
  id, so token and cost attribution needs no guessing.

The runner treats a missing trial or an exception as an infrastructure failure,
not as a model failure.  It reruns only the harnesses whose cell did not reach
a verifier result.  A legitimate reward of 0 is a scored failure and is kept.

Run from WSL (Docker lives there):

    python3 eval/harbor/bench5/run_matrix.py --tag r1

The collector is separate: ``collect.py`` only reads finished jobs and the
proxy log, so a report can be rebuilt without rerunning anything.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request
from collections import Counter
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

import yaml

try:
    from campaign import Campaign, CampaignError, load_campaign
except ImportError:  # when imported as eval.harbor.bench5.run_matrix
    from .campaign import Campaign, CampaignError, load_campaign

try:
    from collect import build_cell as build_audit_cell, load_egress
except ImportError:  # when imported as eval.harbor.bench5.run_matrix
    from .collect import build_cell as build_audit_cell, load_egress

HERE = Path(__file__).resolve().parent
DEFAULT_CAMPAIGN = HERE / "campaigns" / "bench5.yaml"
CANONICAL_AGENT = {
    "litecode": "litecode",
    "codex": "codex",
    "pi": "pi",
    "opencode": "opencode",
    "grok-build": "grok-build",
    "grok": "grok-build",
}


def utc_now() -> str:
    return datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")


def read_key(path: Path) -> str:
    value = path.read_text(encoding="utf-8").strip()
    if not value:
        raise RuntimeError(f"key file is empty: {path}")
    return value


LITECODE_AGENT = "eval.harbor.litecode_agent:LiteCode"


def override_litecode_model(document: dict[str, Any], model: str) -> None:
    """Point only the LiteCode agent at another built-in provider/model.

    The campaign's model stays the baseline for every other harness; this is
    deliberately a post-resolution edit so one harness can change without
    rewriting the shared model layer or the other agents.
    """
    for agent in document.get("agents", []):
        if agent.get("name") == LITECODE_AGENT:
            agent["model_name"] = model
            return
    raise RuntimeError(f"rendered job has no {LITECODE_AGENT} agent")


def task_name(value: str | None) -> str:
    if not value:
        return ""
    return value.rsplit("/", 1)[-1]


class Runner:
    def __init__(self, args: argparse.Namespace) -> None:
        self.args = args
        self.repo = Path(args.repo).resolve()
        self.campaign = load_campaign(args.campaign)
        self.dataset = (
            Path(args.dataset).expanduser().resolve()
            if args.dataset
            else self.campaign.dataset
        )
        self.source_dataset = (
            Path(args.source_dataset).expanduser().resolve()
            if args.source_dataset
            else self.campaign.source_dataset
        )
        self.jobs_root = (
            Path(args.jobs_root).expanduser().resolve()
            if args.jobs_root
            else self.campaign.jobs_root
        )
        self.egress_log = (
            Path(args.egress_log).expanduser().resolve()
            if args.egress_log
            else self.campaign.egress_log
        )
        runtime = args.harness_runtime or self.campaign.harness_runtime
        self.harness_runtime = (
            Path(runtime).expanduser().resolve() if runtime else None
        )
        self.litecode_bin_dir = Path(args.litecode_bin_dir).expanduser().resolve()
        self.litecode_model = args.litecode_model or self.campaign.model
        if "/" not in self.litecode_model:
            raise RuntimeError(
                "--litecode-model must be a provider/model reference, "
                f"got {self.litecode_model!r}"
            )
        provider, model = self.litecode_model.split("/", 1)
        if not provider or not model:
            raise RuntimeError(
                "--litecode-model must be a provider/model reference, "
                f"got {self.litecode_model!r}"
            )
        self.litecode_key = Path(
            args.litecode_key or args.deepseek_key
        ).expanduser().resolve()
        if provider != "deepseek" and not args.litecode_key:
            raise RuntimeError(
                f"--litecode-model {self.litecode_model!r} needs --litecode-key "
                "(a file path, never the key itself)"
            )
        self.registry = self.jobs_root / "registry"
        self.config_dir = self.jobs_root / "configs"
        self.log_dir = self.jobs_root / "logs"
        for directory in (
            self.jobs_root,
            self.registry,
            self.config_dir,
            self.log_dir,
            self.egress_log.parent,
        ):
            directory.mkdir(parents=True, exist_ok=True)

        self.harbor = shutil.which("harbor") or str(
            Path.home() / ".local/bin/harbor"
        )
        self.proxy_url = (
            args.proxy_url or self.campaign.egress_root
        ).rstrip("/")
        if not self.proxy_url:
            raise RuntimeError("campaign has no egress_root and --proxy-url is empty")

        self.tasks = [
            task
            for task in self.campaign.task_names
            if not args.task or task in args.task
        ]
        self.harness_ids = [
            harness
            for harness in self.campaign.harness_ids
            if not args.harness or harness in args.harness
        ]
        unknown_tasks = set(args.task or []) - set(self.campaign.task_names)
        unknown_harnesses = set(args.harness or []) - set(self.campaign.harness_ids)
        if unknown_tasks:
            raise RuntimeError(
                "campaign does not contain task(s): "
                + ", ".join(sorted(unknown_tasks))
            )
        if unknown_harnesses:
            raise RuntimeError(
                "campaign does not contain harness(es): "
                + ", ".join(sorted(unknown_harnesses))
            )
        if not self.tasks:
            raise RuntimeError("no task selected")
        if not self.harness_ids:
            raise RuntimeError("no harness selected")

        self.concurrency = (
            args.concurrency
            if args.concurrency is not None
            else self.campaign.concurrency
        )
        self.retry_rounds = (
            args.retry_rounds
            if args.retry_rounds is not None
            else self.campaign.retry_rounds
        )
        self.job_timeout = (
            args.job_timeout
            if args.job_timeout is not None
            else self.campaign.job_timeout_sec
        )
        self.env = self._environment()
        self.attempts: list[dict[str, Any]] = []
    # ── inputs and preflight ─────────────────────────────────────────────

    def _environment(self) -> dict[str, str]:
        env = os.environ.copy()
        env["HOME"] = str(Path.home())
        env["PATH"] = f"{Path.home()}/.local/bin:" + env.get("PATH", "")
        env["PYTHONPATH"] = str(self.repo) + (
            ":" + env["PYTHONPATH"] if env.get("PYTHONPATH") else ""
        )
        env["PYTHONUNBUFFERED"] = "1"
        env["EVAL_EGRESS_BASE_URL"] = self.proxy_url
        if self.harness_runtime is not None:
            env["HARNESS_RUNTIME"] = str(self.harness_runtime)
        env["LITELLM_LOCAL_MODEL_COST_MAP"] = "True"
        env["DEEPSEEK_API_KEY"] = read_key(Path(self.args.deepseek_key))
        # LiteCode may run on a different provider/model than the baseline
        # harnesses. The adapter turns LITECODE_API_KEY into the credential row
        # for whichever provider the model ref names.
        env["LITECODE_API_KEY"] = read_key(self.litecode_key)
        # The adapter reads the same official key; it is never written to a
        # config file or printed by this runner.
        env["LITECODE_EVAL_BINARY"] = str(self.litecode_bin_dir / "litecode")
        env["LITECODE_EVAL_SEED_DB"] = str(
            self.litecode_bin_dir / "litecode-seed.db"
        )
        env["LITECODE_EVAL_BUILD_INFO"] = str(
            self.litecode_bin_dir / "build-info.json"
        )
        env["LITECODE_EVAL_PROFILE"] = str(self.repo / "eval/profile.toml")
        return env

    def _docker(
        self, *arguments: str, timeout: float | None = None
    ) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["docker", *arguments], capture_output=True, text=True, timeout=timeout
        )

    def _image_has_toolchain(self, image: str) -> tuple[bool, str]:
        probes = [
            harness.probe
            for harness in self.campaign.harnesses
            if harness.probe
        ]
        command = 'set -e; test "$(id -u)" = 0; ' + " ; ".join(probes)
        probe = self._docker(
            "run", "--rm", "--entrypoint", "bash", image, "-lc", command
        )
        detail = (probe.stdout or "") + (probe.stderr or "")
        return probe.returncode == 0, detail.strip()[-1000:]

    def _image_available(self, image: str) -> tuple[bool, str]:
        """Local image or a pullable registry manifest, without pulling it."""
        local = self._docker("image", "inspect", image)
        if local.returncode == 0:
            return True, "local"
        try:
            remote = self._docker("manifest", "inspect", image, timeout=30)
        except subprocess.TimeoutExpired:
            return False, "registry manifest check timed out after 30s"
        detail = ((remote.stdout or "") + (remote.stderr or "")).strip()
        if remote.returncode == 0:
            return True, "registry"
        return False, detail[-1000:] or f"not local and not pullable: {image}"

    def _runtime_probe(self) -> None:
        """Prove the shared runtime runs on every supported task libc."""
        if self.harness_runtime is None:
            return
        probes = [
            harness.probe
            for harness in self.campaign.harnesses
            if harness.probe
        ]
        command = (
            'set -e; test "$(id -u)" = 0; '
            'export PATH="/opt/harness/bin:$PATH"; '
            + " ; ".join(probes)
        )
        for image in ("debian:12-slim", "ubuntu:24.04"):
            probe = self._docker(
                "run",
                "--rm",
                "-e",
                "HOME=/tmp",
                "-v",
                f"{self.harness_runtime}:/opt/harness:ro",
                "--entrypoint",
                "bash",
                image,
                "-lc",
                command,
            )
            detail = ((probe.stdout or "") + (probe.stderr or "")).strip()
            if probe.returncode != 0:
                raise RuntimeError(
                    f"runtime probe failed on {image} with "
                    f"{self.harness_runtime}:\n{detail[-2000:]}"
                )
            print(f"preflight runtime ok: {image}", flush=True)

    def _proxy_health(self) -> None:
        try:
            with urllib.request.urlopen(self.proxy_url + "/healthz", timeout=8) as response:
                payload = json.loads(response.read().decode("utf-8"))
        except (OSError, ValueError, urllib.error.URLError) as error:
            raise RuntimeError(f"proxy health check failed: {error}") from error
        if not payload.get("ok"):
            raise RuntimeError(f"proxy health check returned {payload!r}")

    def _proxy_accounting_probe(self) -> None:
        """Make one tiny generation and prove a usage row lands in this log."""
        suffix = re.sub(r"[^A-Za-z0-9._-]+", "-", self.campaign.name)[:20]
        run_id = f"{suffix}-preflight-{int(time.time())}"[:64]
        body = json.dumps(
            {
                "model": self.campaign.model.rsplit("/", 1)[-1],
                "messages": [
                    {"role": "user", "content": "Reply with exactly: ok"}
                ],
                "stream": False,
                "max_tokens": 8,
            }
        ).encode("utf-8")
        request = urllib.request.Request(
            self.proxy_url + f"/r/{run_id}/v1/chat/completions",
            data=body,
            headers={
                "Content-Type": "application/json",
                "Authorization": "Bearer preflight",
            },
            method="POST",
        )
        try:
            with urllib.request.urlopen(request, timeout=120) as response:
                status = response.status
                response.read()
        except (OSError, urllib.error.URLError) as error:
            raise RuntimeError(f"proxy accounting probe failed: {error}") from error
        if status != 200:
            raise RuntimeError(f"proxy accounting probe returned HTTP {status}")
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            if self.egress_log.is_file():
                for line in self.egress_log.read_text(
                    encoding="utf-8", errors="replace"
                ).splitlines()[-500:]:
                    try:
                        row = json.loads(line)
                    except ValueError:
                        continue
                    if row.get("run_id") == run_id:
                        if row.get("usage") != "ok":
                            raise RuntimeError(
                                "proxy accounting probe row is not usage=ok: "
                                f"{row.get('usage')!r}"
                            )
                        print(
                            "preflight proxy: synthetic generation landed"
                            f" run_id={run_id} prompt={row.get('prompt_tokens')}"
                            f" completion={row.get('completion_tokens')}",
                            flush=True,
                        )
                        return
            time.sleep(0.25)
        raise RuntimeError(
            "proxy is healthy but its accounting row did not appear in "
            f"{self.egress_log}; check that the proxy mounts/writes this path"
        )

    def preflight(self) -> None:
        checks: list[tuple[str, bool, str]] = []
        checks.append(("repo", self.repo.is_dir(), str(self.repo)))
        checks.append(
            (
                "harbor",
                Path(self.harbor).is_file() and os.access(self.harbor, os.X_OK),
                self.harbor,
            )
        )
        checks.append(
            ("source dataset", self.source_dataset.is_dir(), str(self.source_dataset))
        )
        checks.append(("egress log dir", self.egress_log.parent.is_dir(), str(self.egress_log.parent)))
        selected_tasks = [self.campaign.task_map[name] for name in self.tasks]
        for task in selected_tasks:
            ok, detail = self._image_available(task.image)
            checks.append((f"image {task.name}", ok, f"{task.image} ({detail})"))
        if self.harness_runtime is not None:
            checks.append(
                (
                    "harness runtime",
                    self.harness_runtime.is_dir(),
                    str(self.harness_runtime),
                )
            )
            checks.append(
                (
                    "harness runtime version",
                    (self.harness_runtime / "VERSION").is_file(),
                    str(self.harness_runtime / "VERSION"),
                )
            )
        for name, path in (
            ("litecode binary", self.litecode_bin_dir / "litecode"),
            ("litecode seed", self.litecode_bin_dir / "litecode-seed.db"),
            ("litecode build info", self.litecode_bin_dir / "build-info.json"),
            ("litecode profile", self.repo / "eval/profile.toml"),
            ("deepseek key", Path(self.args.deepseek_key)),
        ):
            checks.append((name, path.is_file(), str(path)))

        free_gb = shutil.disk_usage(self.jobs_root.parent).free / (1024**3)
        checks.append(
            (
                "free disk",
                free_gb >= self.campaign.min_free_gb,
                f"{free_gb:.1f}GB free, need {self.campaign.min_free_gb:.1f}GB",
            )
        )
        if not checks or not all(ok for _, ok, _ in checks):
            lines = [
                f"preflight: {name}: {'ok' if ok else 'MISSING'} ({detail})"
                for name, ok, detail in checks
            ]
            raise RuntimeError("\n".join(lines))

        harbor_version = subprocess.run(
            [self.harbor, "--version"], capture_output=True, text=True
        ).stdout.strip()
        if not harbor_version.startswith("0.25."):
            raise RuntimeError(
                f"pinned Harbor 0.25.x required, got {harbor_version!r}"
            )

        info = json.loads((self.litecode_bin_dir / "build-info.json").read_text())
        if info.get("dirty") and not self.args.allow_dirty:
            raise RuntimeError(
                f"litecode build-info says dirty: {self.litecode_bin_dir / 'build-info.json'}; "
                "rebuild from a clean checkout or pass --allow-dirty"
            )
        head = subprocess.run(
            ["git", "-C", str(self.repo), "rev-parse", "HEAD"],
            capture_output=True,
            text=True,
        ).stdout.strip()
        if head and info.get("commit") != head:
            print(
                "preflight warning:"
                f" litecode binary commit {info.get('commit', '?')[:12]}"
                f" != repo HEAD {head[:12]}; the eval profile may not match",
                flush=True,
            )

        if self.harness_runtime is not None:
            if not self.args.skip_image_probe:
                self._runtime_probe()
        elif not self.args.skip_image_probe:
            for task in selected_tasks:
                ok, detail = self._image_has_toolchain(task.image)
                if not ok:
                    raise RuntimeError(
                        f"image toolchain probe failed for {task.image}:\n{detail}"
                    )
                print(f"preflight image ok: {task.image}", flush=True)

        self._proxy_health()
        if not self.args.skip_proxy_probe:
            self._proxy_accounting_probe()

        print(
            "preflight ok:"
            f" campaign={self.campaign.name}"
            f" tasks={len(self.campaign.tasks)} harnesses={len(self.campaign.harnesses)}"
            f" expected_cells={len(self.campaign.expected_cells)}"
            f" harbor={harbor_version}"
            f" litecode={info.get('commit', '?')[:12]} dirty={info.get('dirty')}"
            f" litecode_model={self.litecode_model}"
            f" runtime={self.harness_runtime or '-'}"
            f" free={free_gb:.1f}GB proxy={self.proxy_url}",
            flush=True,
        )

    @staticmethod
    def _tree_sha256(root: Path) -> str:
        digest = hashlib.sha256()
        for path in sorted(root.rglob("*")):
            if not path.is_file():
                continue
            digest.update(path.relative_to(root).as_posix().encode("utf-8"))
            digest.update(b"\0")
            digest.update(path.read_bytes())
        return digest.hexdigest()

    def prepare_dataset(self) -> None:
        """Copy each campaign task and point task.toml at its campaign image.

        Only the environment image changes. Instruction, tests, solution and
        timeouts are copied from the source task; a checksum of the source tree
        is written to the manifest so a later run can refuse a stale copy
        without `--refresh-dataset`.
        """
        old_manifest: dict[str, Any] = {}
        manifest_path = self.dataset / "manifest.json"
        if manifest_path.is_file():
            try:
                old_manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
            except ValueError:
                old_manifest = {}
        old_tasks = {
            item.get("task"): item
            for item in old_manifest.get("tasks") or []
            if isinstance(item, dict)
        }

        records: list[dict[str, Any]] = []
        for task in self.campaign.tasks:
            src = self.source_dataset / task.name
            dst = self.dataset / task.name
            if not src.is_dir():
                raise RuntimeError(f"missing source task: {src}")
            source_toml = src / "task.toml"
            if not source_toml.is_file():
                raise RuntimeError(f"missing {source_toml}")
            source_text = source_toml.read_text(encoding="utf-8")
            match = re.search(r'^docker_image\s*=\s*"([^"]+)"\s*$', source_text, re.M)
            if not match:
                raise RuntimeError(f"{source_toml} has no environment.docker_image line")
            original_image = match.group(1)
            if task.source_image and original_image != task.source_image:
                raise RuntimeError(
                    f"source image drift for {task.name}: campaign says "
                    f"{task.source_image!r}, task.toml says {original_image!r}; "
                    "update the campaign or the source task"
                )
            source_sha = self._tree_sha256(src)
            previous = old_tasks.get(task.name) or {}
            if dst.exists() and not self.args.refresh_dataset:
                if previous.get("source_sha256") is None:
                    raise RuntimeError(
                        f"dataset copy for {task.name} has no source checksum; "
                        "rerun with --refresh-dataset to establish provenance"
                    )
                if previous.get("source_sha256") != source_sha:
                    raise RuntimeError(
                        f"stale dataset copy for {task.name}: source changed; "
                        "rerun with --refresh-dataset"
                    )
            if self.args.refresh_dataset and dst.exists():
                shutil.rmtree(dst)
            if not dst.exists():
                shutil.copytree(src, dst)
            toml = dst / "task.toml"
            text = toml.read_text(encoding="utf-8")
            dst_match = re.search(r'^docker_image\s*=\s*"([^"]+)"\s*$', text, re.M)
            if not dst_match:
                raise RuntimeError(f"{toml} has no environment.docker_image line")
            text = (
                text[: dst_match.start()]
                + f'docker_image = "{task.image}"'
                + text[dst_match.end() :]
            )
            toml.write_text(text, encoding="utf-8")
            records.append(
                {
                    "task": task.name,
                    "source": str(src),
                    "dataset": str(dst),
                    "source_image": original_image,
                    "image": task.image,
                    "runtime": (
                        str(self.harness_runtime)
                        if self.harness_runtime is not None
                        else None
                    ),
                    "source_sha256": source_sha,
                    "instruction_sha256": hashlib.sha256(
                        (src / "instruction.md").read_bytes()
                    ).hexdigest()
                    if (src / "instruction.md").is_file()
                    else None,
                }
            )
        manifest_path.write_text(
            json.dumps(
                {
                    "campaign": self.campaign.name,
                    "campaign_path": str(self.campaign.path),
                    "prepared_at": utc_now(),
                    "tasks": records,
                },
                indent=2,
                ensure_ascii=False,
            )
            + "\n",
            encoding="utf-8",
        )
        print(
            f"dataset ready: {self.dataset} ({len(records)} tasks, "
            f"source checksums recorded)",
            flush=True,
        )
    # ── job rendering and execution ──────────────────────────────────────

    def render(self, task: str, run_id: str, harnesses: list[str]) -> Path:
        command = [
            sys.executable,
            str(self.repo / "eval/harbor/configs/resolve.py"),
            "--job",
            "compare4",
            "--dataset",
            str(self.dataset),
            "--run-id",
            run_id,
            "--base-url",
            self.proxy_url,
            "--task",
            task,
            "--jobs-dir",
            str(self.jobs_root),
            "--concurrency",
            str(min(self.concurrency, len(harnesses))),
        ]
        if self.harness_runtime is not None:
            command.extend(["--harness-runtime", str(self.harness_runtime)])
        for harness in harnesses:
            command.extend(["--harness", harness])
        rendered = subprocess.run(
            command,
            cwd=self.repo,
            env=self.env,
            capture_output=True,
            text=True,
        )
        if rendered.returncode != 0:
            raise RuntimeError(
                f"resolver failed for {run_id}: {rendered.stderr.strip()}"
            )
        document = yaml.safe_load(rendered.stdout)
        # The campaign's model is the stable baseline for every harness. A
        # LiteCode-only override changes one agent after resolution so the
        # other harnesses keep the baseline they were measured on.
        override_litecode_model(document, self.litecode_model)
        path = self.config_dir / f"{run_id}.yaml"
        path.write_text(
            yaml.safe_dump(document, sort_keys=False), encoding="utf-8"
        )
        return path

    def _ensure_image(self, image: str) -> None:
        if self._docker("image", "inspect", image).returncode == 0:
            return
        print(f"pull: {image}", flush=True)
        pull = self._docker("pull", image)
        detail = ((pull.stdout or "") + (pull.stderr or "")).strip()
        if pull.returncode != 0:
            raise RuntimeError(f"docker pull failed for {image}:\n{detail[-2000:]}")

    def _prune_image(self, image: str) -> None:
        for attempt in range(3):
            result = self._docker("image", "rm", image)
            if result.returncode == 0:
                print(f"pruned task image: {image}", flush=True)
                return
            if attempt < 2:
                time.sleep(1)
        detail = ((result.stdout or "") + (result.stderr or "")).strip()
        print(
            f"warning: could not prune task image {image}: {detail[-500:]}",
            flush=True,
        )

    def run_job(self, task: str, run_id: str, harnesses: list[str]) -> dict[str, Any]:
        config = self.render(task, run_id, harnesses)
        log = self.log_dir / f"{run_id}.log"
        started_at = utc_now()
        start = time.time()
        with log.open("w", encoding="utf-8") as handle:
            process = subprocess.run(
                [self.harbor, "run", "-c", str(config)],
                cwd=self.repo,
                env=self.env,
                stdout=handle,
                stderr=subprocess.STDOUT,
                timeout=self.job_timeout,
            )
        elapsed = time.time() - start
        record: dict[str, Any] = {
            "run_id": run_id,
            "task": task,
            "harnesses": harnesses,
            "config": str(config),
            "log": str(log),
            "started_at": started_at,
            "finished_at": utc_now(),
            "elapsed_sec": round(elapsed, 1),
            "returncode": process.returncode,
            "job_dir": None,
            "cells": {},
        }
        self.attempts.append(record)
        print(
            f"  harbor exit={process.returncode} elapsed={elapsed:.0f}s"
            f" log={log}",
            flush=True,
        )
        if process.returncode != 0:
            print(f"  last lines of {log}:", flush=True)
            tail = log.read_text(encoding="utf-8", errors="replace").splitlines()[-20:]
            for line in tail:
                print(f"    {line}", flush=True)
        return record

    # ── result inspection and repair ─────────────────────────────────────

    def find_job_dir(self, run_id: str, started_epoch: float) -> Path | None:
        exact = self.jobs_root / run_id
        if (exact / "result.json").is_file():
            return exact
        candidates: list[tuple[float, Path]] = []
        for result in self.jobs_root.rglob("result.json"):
            try:
                if result.stat().st_mtime < started_epoch - 5:
                    continue
                config = result.parent / "config.json"
                if not config.is_file():
                    continue
                payload = json.loads(config.read_text(encoding="utf-8"))
                if payload.get("job_name") == run_id:
                    candidates.append((result.stat().st_mtime, result.parent))
            except (OSError, ValueError):
                continue
        if not candidates:
            return None
        return max(candidates, key=lambda item: item[0])[1]

    @staticmethod
    def harness_of_trial(trial: dict[str, Any]) -> str | None:
        info = trial.get("agent_info") or {}
        name = info.get("name") or ""
        return CANONICAL_AGENT.get(name, name or None)

    def inspect_result(self, record: dict[str, Any]) -> dict[str, dict[str, Any]]:
        started = datetime.fromisoformat(
            record["started_at"].replace("Z", "+00:00")
        ).timestamp()
        job_dir = self.find_job_dir(record["run_id"], started)
        record["job_dir"] = str(job_dir) if job_dir else None
        if job_dir is None:
            print("  no job directory found for this attempt", flush=True)
            return {}
        # The job-level result.json is only a stats rollup in Harbor 0.25.0.
        # The per-trial facts live in <job_dir>/<trial>/result.json; reading the
        # rollup is exactly the hole that made the previous panel look empty.
        result_files = sorted(job_dir.glob("*/result.json"))
        if not result_files:
            print(f"  no per-trial result.json under {job_dir}", flush=True)
            return {}
        config_path = job_dir / "config.json"
        try:
            job_config = json.loads(config_path.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            job_config = {}
        egress_rows = load_egress(self.egress_log)
        cells: dict[str, dict[str, Any]] = {}
        for result_path in result_files:
            try:
                trial = json.loads(result_path.read_text(encoding="utf-8"))
            except (OSError, ValueError):
                continue
            if not isinstance(trial, dict):
                continue
            harness = self.harness_of_trial(trial)
            if not harness:
                continue
            task = task_name(trial.get("task_name")) or record["task"]
            exception = trial.get("exception_info")
            verifier = trial.get("verifier_result")
            rewards = (verifier or {}).get("rewards") if isinstance(verifier, dict) else None
            valid = isinstance(rewards, dict) and bool(rewards)
            clean = exception is None
            cell = {
                "harness": harness,
                "task": task,
                "model": self.litecode_model
                if harness == "litecode"
                else self.campaign.model,
                "trial": trial.get("trial_name"),
                "valid": valid,
                "clean": clean,
                "reward": rewards,
                "exception": (exception or {}).get("exception_type")
                if isinstance(exception, dict)
                else exception,
                "agent_result": trial.get("agent_result"),
                "agent_info": trial.get("agent_info"),
                "job_dir": str(job_dir),
                "result_json": str(result_path),
                "config_json": str(job_dir / "config.json"),
            }
            spec = self.campaign.harness_map.get(harness)
            audit = build_audit_cell(
                task=task,
                harness=harness,
                trial=trial,
                job_name=job_config.get("job_name", record["run_id"]),
                job_dir=job_dir,
                config=job_config,
                egress=egress_rows,
                strict=self.campaign.strict,
                token_source=spec.token_source if spec else "egress",
            )
            if audit is not None:
                cell["rankable"] = audit.rankable
                cell["audit_reasons"] = audit.audit_reasons
            else:
                cell["rankable"] = False
                cell["audit_reasons"] = ["collector could not build this cell"]
            previous = cells.get(harness)
            if previous is None or (
                cell["valid"]
                and (
                    not previous["valid"]
                    or (cell["rankable"] and not previous.get("rankable"))
                    or (
                        cell["rankable"] == previous.get("rankable")
                        and cell["clean"]
                        and not previous["clean"]
                    )
                )
            ):
                cells[harness] = cell
        record["cells"] = cells
        for harness in record["harnesses"]:
            cell = cells.get(harness)
            if cell is None:
                print(f"  {harness}: MISSING (no trial result)", flush=True)
            elif not cell["valid"]:
                print(
                    f"  {harness}: NO VERIFIER"
                    f" exception={cell['exception']} reward={cell['reward']}",
                    flush=True,
                )
            elif cell["clean"]:
                print(
                    f"  {harness}: reward={cell['reward']}"
                    f" rankable={cell.get('rankable')}"
                    f" trial={cell['trial']}",
                    flush=True,
                )
            else:
                print(
                    f"  {harness}: reward={cell['reward']} but exception="
                    f"{cell['exception']} rankable={cell.get('rankable')}"
                    f" trial={cell['trial']}",
                    flush=True,
                )
            if cell.get("audit_reasons"):
                print(
                    "    audit: " + "; ".join(cell["audit_reasons"]),
                    flush=True,
                )
        return cells
    def write_attempt(self, record: dict[str, Any]) -> None:
        path = self.registry / "attempts.jsonl"
        with path.open("a", encoding="utf-8") as handle:
            handle.write(json.dumps(record, ensure_ascii=False) + "\n")

    @staticmethod
    def _better_cell(candidate: dict[str, Any], previous: dict[str, Any]) -> bool:
        def score(cell: dict[str, Any]) -> tuple[int, int, int]:
            return (
                1 if cell.get("rankable") else 0,
                1 if cell.get("valid") else 0,
                1 if cell.get("clean") else 0,
            )

        return score(candidate) > score(previous)
    def claim_campaign_root(self) -> None:
        """Refuse to mix two campaigns in one jobs root.

        The manifest's jobs_root is a campaign boundary, not a scratch dir.
        Keeping a small campaign.json makes that auditable after the run.
        """
        path = self.jobs_root / "campaign.json"
        record = {
            "campaign": self.campaign.name,
            "campaign_path": str(self.campaign.path),
            "model": self.campaign.model,
            "litecode_model": self.litecode_model,
            "expected_cells": sorted(self.campaign.expected_cells),
            "price_usd_per_million": self.campaign.price,
            "strict": self.campaign.strict,
            "concurrency": self.concurrency,
            "retry_rounds": self.retry_rounds,
            "claimed_at": utc_now(),
        }
        if path.is_file():
            existing = json.loads(path.read_text(encoding="utf-8"))
            if existing.get("campaign") != self.campaign.name:
                raise RuntimeError(
                    f"jobs_root {self.jobs_root} belongs to campaign "
                    f"{existing.get('campaign')!r}, not {self.campaign.name!r}"
                )
            existing.update(record)
            record = existing
        path.write_text(
            json.dumps(record, indent=2, ensure_ascii=False) + "\n",
            encoding="utf-8",
        )
    def strict_audit(self) -> bool:
        """Run the collector's expected-cell/token/trajectory gate.

        The runner only manages Harbor jobs; the collector owns the accounting
        definition. Running it here means a campaign cannot exit 0 while a
        cell is missing token data or a trajectory, even if the verifier passed.
        """
        if self.args.task or self.args.harness:
            print(
                "strict audit skipped for a filtered repair run; run the "
                "collector over the full campaign after repairs",
                flush=True,
            )
            return True
        out_md = self.registry / "audit.md"
        out_json = self.registry / "audit.json"
        command = [
            sys.executable,
            str(self.repo / "eval/harbor/bench5/collect.py"),
            "--campaign",
            str(self.campaign.path),
            "--jobs-root",
            str(self.jobs_root),
            "--egress",
            str(self.egress_log),
            "--out-md",
            str(out_md),
            "--out-json",
            str(out_json),
        ]
        process = subprocess.run(
            command,
            cwd=self.repo,
            env=self.env,
            capture_output=True,
            text=True,
        )
        if process.stdout.strip():
            print(process.stdout.strip(), flush=True)
        if process.returncode != 0:
            print(
                "strict audit failed; see " + str(out_md),
                file=sys.stderr,
            )
            if process.stderr.strip():
                print(process.stderr.strip()[-2000:], file=sys.stderr)
            return False
        print(f"strict audit ok: {out_json}", flush=True)
        return True
    def run(self) -> int:
        self.claim_campaign_root()
        self.preflight()
        self.prepare_dataset()

        selected: dict[str, dict[str, Any]] = {}
        tasks = list(self.tasks)
        requested = list(self.harness_ids)
        if not tasks:
            raise RuntimeError("no task selected")
        if not requested:
            raise RuntimeError("no harness selected")

        failures: list[tuple[str, str, str]] = []
        exception_cells: list[str] = []
        unrankable_cells: list[str] = []
        task_specs = [self.campaign.task_map[name] for name in tasks]
        image_users = Counter(spec.image for spec in task_specs)
        for spec in task_specs:
            task = spec.name
            self._ensure_image(spec.image)
            pending = list(requested)
            best: dict[str, dict[str, Any]] = {}
            for round_number in range(0, self.retry_rounds + 1):
                if not pending:
                    break
                attempt_tag = (
                    self.args.tag
                    if round_number == 0
                    else f"{self.args.tag}-r{round_number + 1}"
                )
                if self.args.harness or self.args.task:
                    # A filtered run is a repair, not a different panel; keep
                    # the marker inside the campaign's 64-character budget.
                    attempt_tag = f"{attempt_tag}-fix"
                run_id = self.campaign.run_id(task, attempt_tag)
                config_path = self.config_dir / f"{run_id}.yaml"
                if config_path.exists() and not self.args.allow_reuse_tag:
                    raise RuntimeError(
                        f"run id {run_id!r} already exists under {self.jobs_root}; "
                        "use a new --tag (or --allow-reuse-tag deliberately)"
                    )
                print(f"\n=== {run_id} ({len(pending)} harnesses) ===", flush=True)
                record = self.run_job(task, run_id, pending)
                cells = self.inspect_result(record)
                self.write_attempt(record)
                for harness in list(pending):
                    cell = cells.get(harness)
                    if not cell or not cell["valid"]:
                        continue
                    previous = best.get(harness)
                    if previous is None or self._better_cell(cell, previous):
                        best[harness] = cell
                    # Only a rankable cell is done. A verifier-passing cell
                    # with missing egress/trajectory goes back through the
                    # retry loop instead of silently entering the ranking.
                    if cell.get("rankable"):
                        pending.remove(harness)
            # `best` includes rankable cells that were removed from `pending`
            # immediately, so the final selection must iterate best, not just
            # the leftover pending harnesses.
            for harness, cell in list(best.items()):
                if not cell or not cell["valid"]:
                    continue
                selected[f"{task}/{harness}"] = cell
                pending = [item for item in pending if item != harness]
                if not cell["clean"]:
                    exception_cells.append(
                        f"{task}/{harness}:{cell['exception']}"
                    )
                if not cell.get("rankable"):
                    unrankable_cells.append(
                        f"{task}/{harness}: {cell.get('audit_reasons')}"
                    )
            for harness in pending:
                failures.append(
                    (task, harness, "no verifier result after retries")
                )
            image_users[spec.image] -= 1
            if (
                self.harness_runtime is not None
                and not self.args.keep_task_images
                and image_users[spec.image] == 0
            ):
                self._prune_image(spec.image)
        self.write_attempt({"finished_at": utc_now(), "selected": selected})
        (self.registry / "selected.json").write_text(
            json.dumps(selected, indent=2, ensure_ascii=False) + "\n",
            encoding="utf-8",
        )

        total = len(tasks) * len(requested)
        complete = len(selected)
        print(
            f"\npanel: {complete}/{total} cells have a verifier result"
            f" ({len(exception_cells)} passed with a harness exception,"
            f" {len(unrankable_cells)} unrankable,"
            f" {len(failures)} missing verifier results)",
            flush=True,
        )
        for detail in exception_cells:
            print(f"  verifier passed but harness exited non-zero: {detail}", flush=True)
        for detail in unrankable_cells:
            print(f"  unrankable: {detail}", flush=True)
        if failures:
            for task, harness, reason in failures:
                print(f"  {task}/{harness}: {reason}", flush=True)
        audit_ok = self.strict_audit()
        return 0 if audit_ok and not failures and not unrankable_cells else 1


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    home = Path.home()
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument(
        "--campaign",
        default=str(DEFAULT_CAMPAIGN),
        help="campaign manifest: tasks, harnesses, paths, prices and gates",
    )
    parser.add_argument("--repo", default=str(Path(__file__).resolve().parents[3]))
    parser.add_argument("--dataset", default=None)
    parser.add_argument("--source-dataset", default=None)
    parser.add_argument("--jobs-root", default=None)
    parser.add_argument("--egress-log", default=None)
    parser.add_argument(
        "--litecode-bin-dir",
        default=str(home / "build/litecode-src/eval/harbor/bin"),
    )
    parser.add_argument("--deepseek-key", default=str(home / "eval/deepseek_key"))
    parser.add_argument(
        "--litecode-model",
        default=None,
        help=(
            "LiteCode-only provider/model ref (for example "
            "cline/cline-pass/deepseek-v4.1-flash); other harnesses keep the "
            "campaign baseline"
        ),
    )
    parser.add_argument(
        "--litecode-key",
        default=None,
        help=(
            "file holding the LiteCode provider key; required when "
            "--litecode-model names a provider other than deepseek"
        ),
    )
    parser.add_argument(
        "--harness-runtime",
        default=None,
        help="shared runtime directory; overrides campaign.harness_runtime",
    )
    parser.add_argument(
        "--keep-task-images",
        action="store_true",
        help="do not prune a task image after its job finishes",
    )
    parser.add_argument(
        "--proxy-url",
        default=os.environ.get("EVAL_EGRESS_BASE_URL"),
        help="override campaign.egress_root",
    )
    parser.add_argument("--tag", default="r1", help="attempt tag, one path segment")
    parser.add_argument("--concurrency", type=int, default=None)
    parser.add_argument("--retry-rounds", type=int, default=None)
    parser.add_argument("--job-timeout", type=int, default=None)
    parser.add_argument("--task", action="append", default=None)
    parser.add_argument("--harness", action="append", default=None)
    parser.add_argument("--refresh-dataset", action="store_true")
    parser.add_argument(
        "--allow-reuse-tag",
        action="store_true",
        help="allow an existing run id/config (dangerous: old egress rows mix in)",
    )
    parser.add_argument("--allow-dirty", action="store_true")
    parser.add_argument(
        "--skip-image-probe",
        action="store_true",
        help="skip docker-run toolchain verification of every task image",
    )
    parser.add_argument(
        "--skip-proxy-probe",
        action="store_true",
        help="skip the tiny synthetic model call that proves accounting works",
    )
    parser.add_argument(
        "--preflight-only",
        action="store_true",
        help="validate images/disk/proxy/accounting and prepare dataset, then exit",
    )
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    if "/" in args.tag:
        raise SystemExit("--tag must be one path segment")
    try:
        runner = Runner(args)
        if args.preflight_only:
            runner.preflight()
            runner.prepare_dataset()
            print("preflight-only: ok", flush=True)
            return 0
        return runner.run()
    except (RuntimeError, OSError, ValueError, CampaignError, subprocess.TimeoutExpired) as error:
        print(f"run_matrix: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())