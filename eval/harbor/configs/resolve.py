#!/usr/bin/env python3
"""Render one Harbor job YAML out of the layers in this directory.

The layers are the four workbuddy-bench names: bench (run invariants),
harnesses (the CLI under test), models (identity + connection), jobs
(the composition). Merge order is bench -> harness -> model -> job, later
wins; a job therefore never repeats a field it does not change.

Two things this script refuses to do, on purpose:

- it will not put a literal endpoint or key in the file it reads: the endpoint
  root comes from EVAL_EGRESS_BASE_URL (or --base-url), the key is a variable
  *name*, and the run id is what tells one harness's traffic from another's;
- it will not let an override of the model's own limits through: a merged job
  that carries context_window / max_output_tokens / max_tokens /
  max_completion_tokens is an error, not a warning.
"""

from __future__ import annotations

import argparse
import copy
import os
import sys
from pathlib import Path
from typing import Any

import yaml

_HERE = Path(__file__).resolve().parent

# A harness number is only comparable if it is the model's own number.
_FORBIDDEN_KEYS = frozenset(
    {"context_window", "max_output_tokens", "max_tokens", "max_completion_tokens"}
)


class ResolveError(Exception):
    """The layers do not make a runnable job; the message says why."""


def _load(path: Path, key: str) -> dict[str, Any]:
    if not path.is_file():
        raise ResolveError(f"missing layer file: {path}")
    document = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
    if not isinstance(document, dict) or not isinstance(document.get(key), dict):
        raise ResolveError(f"{path} must hold a '{key}:' mapping")
    return document[key]


def _merge(base: dict[str, Any], override: dict[str, Any]) -> dict[str, Any]:
    """Deep-merge override into a copy of base; override wins on scalars."""
    merged = copy.deepcopy(base)
    for name, value in override.items():
        if isinstance(value, dict) and isinstance(merged.get(name), dict):
            merged[name] = _merge(merged[name], value)
        else:
            merged[name] = copy.deepcopy(value)
    return merged


def _walk(node: Any):
    if isinstance(node, dict):
        for name, value in node.items():
            yield name
            yield from _walk(value)
    elif isinstance(node, list):
        for value in node:
            yield from _walk(value)


def _check_rendered(rendered: dict[str, Any]) -> None:
    offending = sorted(set(_walk(rendered)) & _FORBIDDEN_KEYS)
    if offending:
        raise ResolveError(
            "overrides the model's own limits: " + ", ".join(offending)
            + " - remove it; the model decides these"
        )
    for agent in rendered["agents"]:
        url = agent.get("base_url")
        if url is not None and ("localhost" in url or "127.0.0.1" in url):
            raise ResolveError(
                f"{agent['name']}: base_url must be the address a container can "
                "reach, not localhost"
            )
        if not agent.get("model_api_key_name"):
            raise ResolveError(f"{agent['name']}: model_api_key_name is empty")


def resolve(
    job: str,
    dataset: str,
    run_id: str,
    egress_base_url: str,
    task_names: list[str] | None = None,
    harness_names: list[str] | None = None,
) -> dict[str, Any]:
    """The Harbor job document for one run of one job file."""
    if not egress_base_url:
        raise ResolveError(
            "no egress root: set EVAL_EGRESS_BASE_URL (http://<bridge>:8399) "
            "or pass --base-url"
        )
    if not dataset:
        raise ResolveError("no dataset: pass --dataset")
    if not run_id or "/" in run_id:
        raise ResolveError("--run-id must be one path segment")

    bench = _load(_HERE / "bench" / "_default.yaml", "bench")
    composition = _load(_HERE / "jobs" / f"{job}.yaml", "job")
    model = _load(_HERE / "models" / f"{composition['model']}.yaml", "model")
    selected_tasks = list(task_names or composition.get("tasks") or [])
    if harness_names:
        # The campaign manifest, not jobs/compare5.yaml, owns the matrix. A
        # harness passed here only needs a layer file; it does not have to be
        # pre-listed in the job composition.
        for name in harness_names:
            if not (_HERE / "harnesses" / f"{name}.yaml").is_file():
                raise ResolveError(f"missing harness layer: {name}.yaml")
        composition["harnesses"] = list(dict.fromkeys(harness_names))

    agents: list[dict[str, Any]] = []
    for harness_key in composition["harnesses"]:
        harness = _load(_HERE / "harnesses" / f"{harness_key}.yaml", "harness")
        harness_id = harness.get("id", harness_key)
        if harness_id != harness_key:
            raise ResolveError(
                f"harnesses/{harness_key}.yaml declares id {harness_id!r}"
            )
        if not isinstance(harness.get("name"), str) or not harness["name"]:
            raise ResolveError(f"{harness_key}: harness name is empty")
        if harness.get("protocol") not in model["protocols"]:
            raise ResolveError(
                f"{harness_key}: protocol {harness.get('protocol')!r} is not one "
                f"of the model's {model['protocols']}"
            )
        # bench -> harness -> model -> job, later wins.
        kwargs = _merge(model.get("params") or {}, harness.get("params") or {})
        kwargs = _merge(kwargs, composition.get("params") or {})
        agent: dict[str, Any] = {
            "name": harness["name"],
            "model_name": model["name"],
            "model_api_key_name": harness.get(
                "model_api_key_name", model["backend_key_env"]
            ),
        }
        # LiteCode resolves its endpoint from its own compiled provider catalog.
        # Every other harness has to point at the one accounting exit, with the
        # run id naming both the job and the harness.
        if harness.get("egress", True):
            # Chat harnesses talk to /v1/chat/completions; Codex appends
            # /responses itself, so its base_url stops at the run id.
            suffix = harness.get("base_url_suffix", "/v1")
            if not isinstance(suffix, str) or not suffix.startswith("/"):
                raise ResolveError(
                    f"{harness_key}: base_url_suffix must start with /"
                )
            agent["base_url"] = (
                egress_base_url.rstrip("/")
                + f"/r/{run_id}-{harness_key}"
                + suffix
            )
        api_format = harness.get("model_api_format")
        if api_format is not None:
            agent["model_api_format"] = api_format
        agent["kwargs"] = kwargs
        agents.append(agent)

    rendered: dict[str, Any] = {
        "job_name": run_id,
        "jobs_dir": bench["jobs_dir"],
        "n_attempts": bench["n_attempts"],
        "n_concurrent_trials": bench["n_concurrent_trials"],
    }
    if "agent_setup_timeout_multiplier" in bench:
        rendered["agent_setup_timeout_multiplier"] = bench[
            "agent_setup_timeout_multiplier"
        ]
    # A job may tighten run-invariant knobs (for example concurrency) without
    # editing the shared bench layer. Absolute locations stay out of the
    # composition files; the runner supplies jobs_dir on the command line.
    for key in (
        "n_attempts",
        "n_concurrent_trials",
        "agent_setup_timeout_multiplier",
        "jobs_dir",
    ):
        if key in composition:
            rendered[key] = composition[key]
    dataset_entry: dict[str, Any] = {"path": dataset}
    if selected_tasks:
        dataset_entry["task_names"] = selected_tasks
    rendered.update(
        {
            "environment": bench["environment"],
            "datasets": [dataset_entry],
            "agents": agents,
        }
    )
    _check_rendered(rendered)
    return rendered


def main(argv: list[str] | None = None) -> int:
    description = (__doc__ or "").split("\n\n")[0]
    parser = argparse.ArgumentParser(description=description)
    parser.add_argument("--job", required=True, help="file under jobs/, without .yaml")
    parser.add_argument("--dataset", required=True, help="dataset path Harbor should run")
    parser.add_argument("--run-id", required=True, help="one path segment; names the run")
    parser.add_argument(
        "--base-url",
        default=os.environ.get("EVAL_EGRESS_BASE_URL", ""),
        help="egress proxy root; defaults to $EVAL_EGRESS_BASE_URL",
    )
    parser.add_argument(
        "--task",
        action="append",
        default=None,
        help="task name to run; repeatable. Defaults to the job layer's tasks.",
    )
    parser.add_argument(
        "--harness",
        action="append",
        default=None,
        help="harness key to include; repeatable. Defaults to the job layer.",
    )
    parser.add_argument(
        "--jobs-dir",
        default=None,
        help="absolute results root for this attempt; overrides the bench layer.",
    )
    parser.add_argument(
        "--concurrency",
        type=int,
        default=None,
        help="override n_concurrent_trials for this attempt.",
    )
    args = parser.parse_args(argv)
    try:
        rendered = resolve(
            args.job,
            args.dataset,
            args.run_id,
            args.base_url,
            task_names=args.task,
            harness_names=args.harness,
        )
        if args.jobs_dir is not None:
            rendered["jobs_dir"] = args.jobs_dir
        if args.concurrency is not None:
            rendered["n_concurrent_trials"] = args.concurrency
    except ResolveError as error:
        print(f"resolve: {error}", file=sys.stderr)
        return 2
    yaml.safe_dump(rendered, sys.stdout, sort_keys=False)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
