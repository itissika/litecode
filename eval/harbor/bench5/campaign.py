"""Campaign manifest: the one place that says what a run contains.

The previous runner and collector hard-coded three task names and five harness
names. That is fine for a one-off panel and a silent data-loss risk as soon as
the panel grows: a new task that is not in the tuple is simply filtered out.
This module makes the matrix data: tasks, harnesses, expected cells, images,
price version and every path come from a YAML file.

Relative paths in a campaign are resolved against the repository root. The
manifest is deliberately strict: duplicate task/harness names, empty matrices,
bad token sources and missing price keys fail before a container is started.
"""

from __future__ import annotations

import hashlib
import os
import re
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import yaml

RUN_ID_LIMIT = 64
_SAFE_RUN = re.compile(r"^[A-Za-z0-9._-]+$")
_SAFE_TASK = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")
_TOKEN_SOURCES = frozenset({"litecode", "egress"})
_DEFAULT_STRICT = {
    "require_expected_cells": True,
    "require_token_complete": True,
    "require_trajectory": True,
    "require_clean_exit": False,
}
_PRICE_KEYS = ("uncached_input", "cached_input", "output")


class CampaignError(Exception):
    """The campaign file cannot describe a runnable matrix."""


@dataclass(frozen=True)
class TaskSpec:
    name: str
    image: str
    source_image: str | None = None
    difficulty: str = "unknown"
    category: str = "unknown"


@dataclass(frozen=True)
class HarnessSpec:
    id: str
    token_source: str
    required_trajectory: bool = True
    probe: str | None = None


@dataclass(frozen=True)
class Campaign:
    path: Path
    name: str
    run_prefix: str
    model: str
    jobs_root: Path
    dataset: Path
    source_dataset: Path
    egress_log: Path
    egress_root: str
    concurrency: int
    retry_rounds: int
    job_timeout_sec: int
    min_free_gb: float
    price: dict[str, float]
    strict: dict[str, bool]
    tasks: tuple[TaskSpec, ...]
    harnesses: tuple[HarnessSpec, ...]

    @property
    def task_names(self) -> tuple[str, ...]:
        return tuple(task.name for task in self.tasks)

    @property
    def harness_ids(self) -> tuple[str, ...]:
        return tuple(harness.id for harness in self.harnesses)

    @property
    def expected_cells(self) -> set[str]:
        return {
            f"{task.name}/{harness.id}"
            for task in self.tasks
            for harness in self.harnesses
        }

    @property
    def task_map(self) -> dict[str, TaskSpec]:
        return {task.name: task for task in self.tasks}

    @property
    def harness_map(self) -> dict[str, HarnessSpec]:
        return {harness.id: harness for harness in self.harnesses}

    def run_id(self, task: str, tag: str) -> str:
        """A run id that leaves room for ``-<harness>`` under the proxy limit."""
        if self.task_map.get(task) is None:
            raise CampaignError(f"task {task!r} is not in campaign {self.name!r}")
        if not _SAFE_RUN.match(tag):
            raise CampaignError(f"tag {tag!r} is not one path segment")
        base = f"{self.run_prefix}-{task}-{tag}"
        reserve = max(len(harness.id) for harness in self.harnesses) + 1
        room = RUN_ID_LIMIT - reserve
        if len(base) > room:
            digest = hashlib.sha1(base.encode("utf-8")).hexdigest()[:8]
            base = base[: room - 9].rstrip("-") + "-" + digest
        if len(base) + reserve > RUN_ID_LIMIT:
            raise CampaignError(
                f"run id {base!r} cannot fit the 64-character proxy limit"
            )
        return base


def _mapping(value: Any, where: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise CampaignError(f"{where} must be a mapping")
    return value


def _list(value: Any, where: str) -> list[Any]:
    if not isinstance(value, list) or not value:
        raise CampaignError(f"{where} must be a non-empty list")
    return value


def _positive_int(value: Any, where: str, default: int) -> int:
    if value is None:
        return default
    if not isinstance(value, int) or isinstance(value, bool) or value < 1:
        raise CampaignError(f"{where} must be a positive integer")
    return value


def _non_negative_int(value: Any, where: str, default: int) -> int:
    if value is None:
        return default
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise CampaignError(f"{where} must be a non-negative integer")
    return value


def _path(value: Any, where: str, repo: Path) -> Path:
    if not isinstance(value, str) or not value.strip():
        raise CampaignError(f"{where} must be a non-empty path")
    path = Path(os.path.expandvars(value)).expanduser()
    if not path.is_absolute():
        path = repo / path
    return path.resolve()


def load_campaign(path: str | Path) -> Campaign:
    campaign_path = Path(path).expanduser().resolve()
    if not campaign_path.is_file():
        raise CampaignError(f"campaign not found: {campaign_path}")
    document = yaml.safe_load(campaign_path.read_text(encoding="utf-8")) or {}
    document = _mapping(document, str(campaign_path))
    if document.get("schema_version") != 1:
        raise CampaignError(f"{campaign_path}: schema_version must be 1")

    repo = Path(__file__).resolve().parents[3]
    name = document.get("name")
    run_prefix = document.get("run_prefix", name)
    model = document.get("model")
    for label, value in (("name", name), ("model", model), ("run_prefix", run_prefix)):
        if not isinstance(value, str) or not value.strip():
            raise CampaignError(f"{campaign_path}: {label} must be a non-empty string")
    if not _SAFE_TASK.match(run_prefix):
        raise CampaignError(f"{campaign_path}: run_prefix has unsafe characters")

    price = _mapping(
        document.get("price_usd_per_million"), "price_usd_per_million"
    )
    parsed_price: dict[str, float] = {}
    for key in _PRICE_KEYS:
        value = price.get(key)
        if not isinstance(value, (int, float)) or isinstance(value, bool) or value < 0:
            raise CampaignError(f"{campaign_path}: price_usd_per_million.{key} is invalid")
        parsed_price[key] = float(value)

    strict = dict(_DEFAULT_STRICT)
    overrides = document.get("strict") or {}
    overrides = _mapping(overrides, "strict")
    for key, value in overrides.items():
        if key not in strict:
            raise CampaignError(f"{campaign_path}: unknown strict key {key!r}")
        if not isinstance(value, bool):
            raise CampaignError(f"{campaign_path}: strict.{key} must be boolean")
        strict[key] = value

    tasks: list[TaskSpec] = []
    for index, raw in enumerate(_list(document.get("tasks"), "tasks")):
        item = _mapping(raw, f"tasks[{index}]")
        task_name = item.get("name")
        image = item.get("image")
        if not isinstance(task_name, str) or not _SAFE_TASK.match(task_name):
            raise CampaignError(f"{campaign_path}: tasks[{index}].name is unsafe")
        if not isinstance(image, str) or not image.strip():
            raise CampaignError(f"{campaign_path}: tasks[{index}].image is empty")
        source_image = item.get("source_image")
        if source_image is not None and (
            not isinstance(source_image, str) or not source_image.strip()
        ):
            raise CampaignError(
                f"{campaign_path}: tasks[{index}].source_image must be a string"
            )
        tasks.append(
            TaskSpec(
                name=task_name,
                image=image,
                source_image=source_image,
                difficulty=str(item.get("difficulty") or "unknown"),
                category=str(item.get("category") or "unknown"),
            )
        )
    if len({task.name for task in tasks}) != len(tasks):
        raise CampaignError(f"{campaign_path}: duplicate task names")

    harnesses: list[HarnessSpec] = []
    for index, raw in enumerate(_list(document.get("harnesses"), "harnesses")):
        item = _mapping(raw, f"harnesses[{index}]")
        harness_id = item.get("id")
        source = item.get("token_source")
        if not isinstance(harness_id, str) or not _SAFE_TASK.match(harness_id):
            raise CampaignError(f"{campaign_path}: harnesses[{index}].id is unsafe")
        if source not in _TOKEN_SOURCES:
            raise CampaignError(
                f"{campaign_path}: harnesses[{index}].token_source must be "
                f"one of {sorted(_TOKEN_SOURCES)}"
            )
        probe = item.get("probe")
        if probe is not None and (not isinstance(probe, str) or not probe.strip()):
            raise CampaignError(
                f"{campaign_path}: harnesses[{index}].probe must be a string"
            )
        if source == "egress" and not probe:
            raise CampaignError(
                f"{campaign_path}: harnesses[{index}] uses egress and needs a "
                "probe command so preflight can verify the image"
            )
        harnesses.append(
            HarnessSpec(
                id=harness_id,
                token_source=source,
                required_trajectory=bool(item.get("required_trajectory", True)),
                probe=probe,
            )
        )
    if len({harness.id for harness in harnesses}) != len(harnesses):
        raise CampaignError(f"{campaign_path}: duplicate harness ids")

    return Campaign(
        path=campaign_path,
        name=name,
        run_prefix=run_prefix,
        model=model,
        jobs_root=_path(document.get("jobs_root"), "jobs_root", repo),
        dataset=_path(document.get("dataset"), "dataset", repo),
        source_dataset=_path(document.get("source_dataset"), "source_dataset", repo),
        egress_log=_path(document.get("egress_log"), "egress_log", repo),
        egress_root=str(document.get("egress_root") or "").rstrip("/"),
        concurrency=_positive_int(document.get("concurrency"), "concurrency", 5),
        retry_rounds=_non_negative_int(
            document.get("retry_rounds"), "retry_rounds", 2
        ),
        job_timeout_sec=_positive_int(
            document.get("job_timeout_sec"), "job_timeout_sec", 5400
        ),
        min_free_gb=float(document.get("min_free_gb", 0) or 0),
        price=parsed_price,
        strict=strict,
        tasks=tuple(tasks),
        harnesses=tuple(harnesses),
    )