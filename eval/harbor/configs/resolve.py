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
        url = agent["base_url"]
        if "localhost" in url or "127.0.0.1" in url:
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

    agents: list[dict[str, Any]] = []
    for harness_name in composition["harnesses"]:
        harness = _load(_HERE / "harnesses" / f"{harness_name}.yaml", "harness")
        if harness.get("name") != harness_name:
            raise ResolveError(
                f"harnesses/{harness_name}.yaml declares name {harness.get('name')!r}"
            )
        if harness.get("protocol") not in model["protocols"]:
            raise ResolveError(
                f"{harness_name}: protocol {harness.get('protocol')!r} is not one "
                f"of the model's {model['protocols']}"
            )
        # bench -> harness -> model -> job, later wins.
        kwargs = _merge(model.get("params") or {}, harness.get("params") or {})
        kwargs = _merge(kwargs, composition.get("params") or {})
        # Chat harnesses talk to /v1/chat/completions; Codex appends /responses
        # itself, so its base_url stops at the run id.
        suffix = harness.get("base_url_suffix", "/v1")
        if not isinstance(suffix, str) or not suffix.startswith("/"):
            raise ResolveError(f"{harness_name}: base_url_suffix must start with /")
        agent: dict[str, Any] = {
            "name": harness_name,
            "model_name": model["name"],
            "base_url": (
                egress_base_url.rstrip("/") + f"/r/{run_id}-{harness_name}" + suffix
            ),
            "model_api_key_name": model["backend_key_env"],
        }
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
    rendered.update(
        {
            "environment": bench["environment"],
            "datasets": [{"path": dataset}],
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
    args = parser.parse_args(argv)
    try:
        rendered = resolve(args.job, args.dataset, args.run_id, args.base_url)
    except ResolveError as error:
        print(f"resolve: {error}", file=sys.stderr)
        return 2
    yaml.safe_dump(rendered, sys.stdout, sort_keys=False)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
