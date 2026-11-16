#!/usr/bin/env python3
"""Build the harness report from finished jobs and the proxy log.

Two accounting sources, never blended:

* litecode: the CLI's own final usage line. The adapter copies the same numbers
  into ``agent_result`` and the trial artifact; both are shown and checked.
* the other four: only the egress proxy's ``usage=ok`` rows. Harness
  self-report is shown as a cross-check, never as the score.

The report is opinionated about missing data: a missing token count stays
missing and is flagged. It is not filled with 0 and it is not estimated from a
sibling harness.

    python3 eval/harbor/bench5/collect.py --jobs-root ~/eval/jobs/bench5

Writes ``~/eval/bench5-report.md`` and ``~/eval/bench5-report.json``.
"""

from __future__ import annotations

import argparse
import json
import re
import statistics
import sys
from collections import Counter, defaultdict
from dataclasses import dataclass, field
from datetime import datetime
from pathlib import Path
from typing import Any

try:
    from campaign import Campaign, CampaignError, load_campaign
    from tool_usage import summarize_reminders, summarize_tool_usage
    from trace import summarize_trace
except ImportError:  # when imported as eval.harbor.bench5.collect
    from .campaign import Campaign, CampaignError, load_campaign
    from .tool_usage import summarize_reminders, summarize_tool_usage
    from .trace import summarize_trace

HERE = Path(__file__).resolve().parent
DEFAULT_CAMPAIGN = HERE / "campaigns" / "bench5.yaml"

# Defaults keep the module importable for small unit tests; build_report()
# overwrites all three from the campaign manifest before reading any jobs.
TASKS = ("prove-plus-comm", "polyglot-c-py", "password-recovery")
HARNESSES = ("litecode", "codex", "pi", "opencode")
CANONICAL_AGENT = {
    "litecode": "litecode",
    "codex": "codex",
    "pi": "pi",
    "opencode": "opencode",
    "grok-build": "grok-build",
    "grok": "grok-build",
}
# The price table the previous round reverse-engineered from real DeepSeek
# bills and then reproduced to six decimals. Keep it here, in one place, so
# every harness is priced by the same arithmetic.
PRICE_UNCACHED_INPUT = 0.15 / 1_000_000
PRICE_CACHED_INPUT = 0.003 / 1_000_000
PRICE_OUTPUT = 0.60 / 1_000_000
USAGE_LINE = re.compile(
    r"tokens prompt=(\d+) completion=(\d+) cache_hit=(\d+) cache_miss=(\d+)"
)
RUN_IN_URL = re.compile(r"/r/([^/]+)")


def read_json(path: Path) -> dict[str, Any] | None:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    return value if isinstance(value, dict) else None


def parse_time(value: Any) -> datetime | None:
    if not isinstance(value, str) or not value:
        return None
    try:
        return datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        return None


def seconds_between(start: Any, end: Any) -> float | None:
    first, last = parse_time(start), parse_time(end)
    if first is None or last is None:
        return None
    return max((last - first).total_seconds(), 0.0)


def token_int(value: Any) -> int | None:
    if isinstance(value, int) and not isinstance(value, bool) and value >= 0:
        return value
    return None


def money(value: float | None) -> str:
    return "缺失" if value is None else f"${value:.6f}"


def num(value: int | float | None) -> str:
    return "缺失" if value is None else f"{int(value):,}"


def k(value: int | float | None) -> str:
    if value is None:
        return "缺失"
    if value < 1000:
        return f"{value:,}"
    return f"{value / 1000:.1f}K"


def pct(value: float | None) -> str:
    return "缺失" if value is None else f"{value * 100:.1f}%"


def cache_trusted_model(model: str | None) -> bool:
    """Only the official provider's cache-hit split is trusted.

    Resellers of DeepSeek report cache hits that move independently of the
    request stream; using that split would silently understate cost. Unknown
    models stay trusted so a missing trace does not change an existing number.
    """
    if not model:
        return True
    return model.split("/", 1)[0] == "deepseek"


def cost_of(prompt: int | None, cached: int | None, completion: int | None) -> float | None:
    if prompt is None or cached is None or completion is None:
        return None
    cached = min(max(cached, 0), prompt)
    uncached = max(prompt - cached, 0)
    return (
        uncached * PRICE_UNCACHED_INPUT
        + cached * PRICE_CACHED_INPUT
        + max(completion, 0) * PRICE_OUTPUT
    )


def cost_parts(
    prompt: int | None, cached: int | None, completion: int | None
) -> dict[str, float | None]:
    if prompt is None or cached is None or completion is None:
        return {"uncached": None, "cached": None, "output": None}
    cached = min(max(cached, 0), prompt)
    return {
        "uncached": max(prompt - cached, 0) * PRICE_UNCACHED_INPUT,
        "cached": cached * PRICE_CACHED_INPUT,
        "output": max(completion, 0) * PRICE_OUTPUT,
    }


def harness_from_name(name: str | None) -> str | None:
    if not name:
        return None
    if name in CANONICAL_AGENT:
        return CANONICAL_AGENT[name]
    tail = name.rsplit(":", 1)[-1].rsplit(".", 1)[-1]
    class_map = {
        "LiteCode": "litecode",
        "PrebakedCodex": "codex",
        "PrebakedPi": "pi",
        "PrebakedOpenCode": "opencode",
        "PrebakedGrokBuild": "grok-build",
    }
    return class_map.get(tail, CANONICAL_AGENT.get(tail.lower(), tail))


def task_from_name(name: str | None) -> str | None:
    if not name:
        return None
    value = name.rsplit("/", 1)[-1]
    return value if value in TASKS else None

def exception_summary(value: Any) -> Any:
    """Keep the exception type and a bounded message, not a 16 KB traceback."""
    if isinstance(value, dict):
        return {
            "type": value.get("exception_type") or "exception",
            "message": str(value.get("exception_message") or "")[:1000],
        }
    return value


def exception_label(value: Any) -> str:
    if isinstance(value, dict):
        return str(value.get("type") or "exception")
    return str(value or "exception")

def read_failure_hint(trial_dir: Path) -> str | None:
    """The last pytest assertion line, short enough to put in the report."""
    path = trial_dir / "verifier/test-stdout.txt"
    try:
        lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError:
        return None
    evidence = [
        line.strip()[2:].strip()
        for line in lines
        if line.strip().startswith("E ") and line.strip()[2:].strip()
    ]
    if not evidence:
        return None
    # Pytest's last E line is usually "Use -v to get more diff"; the assertion
    # itself is the useful diagnosis.
    for needle in ("AssertionError:", "assert "):
        for line in evidence:
            if needle in line:
                return line[:500]
    return evidence[-1][:500]


@dataclass
class EgressRows:
    run_id: str
    rows: list[dict[str, Any]] = field(default_factory=list)

    @property
    def requests(self) -> int:
        return len(self.rows)

    @property
    def ok_rows(self) -> int:
        return sum(1 for row in self.rows if row.get("usage") == "ok")

    @property
    def missing_usage(self) -> int:
        return sum(1 for row in self.rows if row.get("usage") == "missing")

    @property
    def http_errors(self) -> int:
        """Generation calls the upstream rejected before producing usage."""
        return sum(
            1
            for row in self.rows
            if isinstance(row.get("status"), int) and row["status"] >= 400
        )

    @property
    def aborted(self) -> int:
        """2xx generations whose client/stream died before a usage frame."""
        return sum(
            1
            for row in self.rows
            if row.get("usage") == "missing"
            and isinstance(row.get("status"), int)
            and row["status"] < 400
            and bool(row.get("error"))
        )

    @property
    def successful_missing(self) -> int:
        """Completed 2xx generations with no usage: the only blocking hole."""
        return sum(
            1
            for row in self.rows
            if row.get("usage") == "missing"
            and isinstance(row.get("status"), int)
            and row["status"] < 400
            and not row.get("error")
        )

    @property
    def token_complete(self) -> bool:
        return self.ok_rows > 0 and self.successful_missing == 0

    def _sum(self, key: str) -> int:
        return sum(
            int(row.get(key) or 0)
            for row in self.rows
            if row.get("usage") == "ok" and isinstance(row.get(key), int)
        )

    @property
    def prompt(self) -> int:
        return self._sum("prompt_tokens")

    @property
    def cached(self) -> int:
        return self._sum("cached_tokens")

    @property
    def completion(self) -> int:
        return self._sum("completion_tokens")


@dataclass
class Cell:
    task: str
    harness: str
    trial: str | None
    job_name: str
    job_dir: Path
    valid: bool
    clean: bool
    reward: dict[str, Any] | None
    exception: Any
    agent_context: dict[str, Any]
    agent_info: dict[str, Any]
    started_at: str | None
    finished_at: str | None
    agent_setup_sec: float | None
    agent_execution_sec: float | None
    verifier_sec: float | None
    token_source: str
    token_trusted: bool
    rankable: bool = False
    audit_reasons: list[str] = field(default_factory=list)
    aborted_requests: int = 0
    http_error_requests: int = 0
    prompt: int | None = None
    cached: int | None = None
    completion: int | None = None
    requests: int | None = None
    egress_run_id: str | None = None
    egress: EgressRows | None = None
    harbor_prompt: int | None = None
    harbor_cached: int | None = None
    harbor_completion: int | None = None
    harbor_cost: float | None = None
    litecode_line: tuple[int, int, int, int] | None = None
    token_note: str = ""
    failure_hint: str | None = None
    trajectory: dict[str, Any] = field(default_factory=dict)
    tool_usage: dict[str, Any] = field(default_factory=dict)
    trace: dict[str, Any] = field(default_factory=dict)
    artifacts: list[dict[str, Any]] = field(default_factory=list)
    cost: float | None = None
    cost_breakdown: dict[str, float | None] = field(default_factory=dict)
    model: str | None = None
    cache_trusted: bool = True
    reported_cost: float | None = None

    @property
    def passed(self) -> bool:
        if not self.valid or not self.reward:
            return False
        return any(float(value) >= 0.5 for value in self.reward.values())

    @property
    def total_tokens(self) -> int | None:
        if self.prompt is None or self.completion is None:
            return None
        return self.prompt + self.completion

    @property
    def cache_rate(self) -> float | None:
        if not self.prompt:
            return None
        return (self.cached or 0) / self.prompt

    def to_json(self) -> dict[str, Any]:
        return {
            "task": self.task,
            "harness": self.harness,
            "model": self.model,
            "trial": self.trial,
            "job_name": self.job_name,
            "job_dir": str(self.job_dir),
            "valid": self.valid,
            "clean": self.clean,
            "passed": self.passed,
            "rankable": self.rankable,
            "audit_reasons": self.audit_reasons,
            "aborted_requests": self.aborted_requests,
            "http_error_requests": self.http_error_requests,
            "reward": self.reward,
            "exception": self.exception,
            "token_source": self.token_source,
            "token_trusted": self.token_trusted,
            "tokens": {
                "prompt_including_cache": self.prompt,
                "cached": self.cached,
                "output": self.completion,
                "total": self.total_tokens,
                "cache_rate": self.cache_rate,
            },
            "requests": self.requests,
            "egress_run_id": self.egress_run_id,
            "harbor_tokens": {
                "prompt_including_cache": self.harbor_prompt,
                "cached": self.harbor_cached,
                "output": self.harbor_completion,
                "cost_usd": self.harbor_cost,
            },
            "litecode_usage_line": list(self.litecode_line)
            if self.litecode_line
            else None,
            "token_note": self.token_note,
            "failure_hint": self.failure_hint,
            "cost_usd": self.cost,
            "reported_cost_usd": self.reported_cost,
            "cache_trusted": self.cache_trusted,
            "cost_breakdown": self.cost_breakdown,
            "timing_sec": {
                "agent_setup": self.agent_setup_sec,
                "agent_execution": self.agent_execution_sec,
                "verifier": self.verifier_sec,
                "trial_total": seconds_between(self.started_at, self.finished_at),
            },
            "trajectory": self.trajectory,
            "tool_usage": self.tool_usage,
            "trace": self.trace,
            "artifacts": self.artifacts,
            "egress_requests": [
                {
                    "ts": row.get("ts"),
                    "request_id": row.get("request_id"),
                    "prompt_tokens": row.get("prompt_tokens"),
                    "cached_tokens": row.get("cached_tokens"),
                    "completion_tokens": row.get("completion_tokens"),
                    "duration_ms": row.get("duration_ms"),
                    "status": row.get("status"),
                    "stream": row.get("stream"),
                    "usage": row.get("usage"),
                    "path": row.get("path"),
                    "messages_count": row.get("messages_count"),
                    "tools_count": row.get("tools_count"),
                }
                for row in (self.egress.rows if self.egress else [])
            ],
        }

def forward_run_ids(config: dict[str, Any]) -> dict[str, str]:
    result: dict[str, str] = {}
    for agent in config.get("agents") or []:
        if not isinstance(agent, dict):
            continue
        harness = harness_from_name(agent.get("name"))
        url = agent.get("base_url")
        if harness and isinstance(url, str):
            match = RUN_IN_URL.search(url)
            if match:
                result[harness] = match.group(1)
    return result


def load_egress(path: Path) -> dict[str, EgressRows]:
    buckets: dict[str, EgressRows] = defaultdict(lambda: EgressRows(""))
    if not path.is_file():
        return {}
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        try:
            row = json.loads(line)
        except ValueError:
            continue
        if not isinstance(row, dict):
            continue
        run_id = row.get("run_id")
        if not isinstance(run_id, str):
            continue
        path_value = row.get("path")
        if row.get("usage") == "n/a" and not (
            isinstance(path_value, str)
            and path_value.endswith(("/chat/completions", "/responses", "/completions"))
        ):
            continue
        bucket = buckets[run_id]
        bucket.run_id = run_id
        bucket.rows.append(row)
    return dict(buckets)


def summarize_trajectory(path: Path) -> dict[str, Any]:
    document = read_json(path)
    if not document:
        return {"present": False, "reminders": summarize_reminders([])}
    steps = document.get("steps")
    if not isinstance(steps, list):
        steps = []
    tool_counts: Counter[str] = Counter()
    observations = 0
    llm_calls = 0
    sources: Counter[str] = Counter()
    for step in steps:
        if not isinstance(step, dict):
            continue
        sources[str(step.get("source") or "?")] += 1
        for call in step.get("tool_calls") or []:
            if isinstance(call, dict):
                tool_counts[
                    str(call.get("function_name") or call.get("name") or "?")
                ] += 1
        observation = step.get("observation")
        if isinstance(observation, dict):
            observations += len(observation.get("results") or [])
        count = step.get("llm_call_count")
        if isinstance(count, int) and count >= 0:
            llm_calls += count
        elif step.get("source") == "agent":
            llm_calls += 1
    final_metrics = document.get("final_metrics")
    return {
        "present": True,
        "schema_version": document.get("schema_version"),
        "n_steps": len(steps),
        "sources": dict(sources),
        "n_tool_calls": sum(tool_counts.values()),
        "tool_calls": dict(tool_counts.most_common()),
        "n_observations": observations,
        "llm_calls": llm_calls,
        "final_metrics": final_metrics if isinstance(final_metrics, dict) else None,
        "reminders": summarize_reminders(steps),
    }


def litecode_usage_line(text: str | None) -> tuple[int, int, int, int] | None:
    if not text:
        return None
    matches = USAGE_LINE.findall(text)
    if not matches:
        return None
    prompt, completion, cached, cache_miss = matches[-1]
    return int(prompt), int(completion), int(cached), int(cache_miss)


def litecode_requests(trial_dir: Path, trajectory: dict[str, Any]) -> int | None:
    value = trajectory.get("llm_calls")
    if isinstance(value, int) and value > 0:
        return value
    log = trial_dir / "agent/litecode/logs/litecode.log"
    try:
        text = log.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return None
    count = text.count('"message":"agent loop iteration"')
    return count or None


def read_artifacts(trial_dir: Path) -> list[dict[str, Any]]:
    root = trial_dir / "agent"
    if not root.is_dir():
        return []
    files: list[dict[str, Any]] = []
    for path in sorted(root.rglob("*")):
        if not path.is_file():
            continue
        try:
            size = path.stat().st_size
        except OSError:
            continue
        files.append({"path": path.relative_to(trial_dir).as_posix(), "bytes": size})
    return files


def build_cell(
    *,
    task: str,
    harness: str,
    trial: dict[str, Any],
    job_name: str,
    job_dir: Path,
    config: dict[str, Any],
    egress: dict[str, EgressRows],
    strict: dict[str, bool] | None = None,
    token_source: str = "egress",
) -> Cell | None:
    trial_name = trial.get("trial_name")
    if not isinstance(trial_name, str):
        return None
    trial_dir = job_dir / trial_name
    agent_context = trial.get("agent_result")
    if not isinstance(agent_context, dict):
        agent_context = {}
    agent_info = trial.get("agent_info")
    if not isinstance(agent_info, dict):
        agent_info = {}
    trial_config = trial.get("config")
    agent_config = (
        trial_config.get("agent") if isinstance(trial_config, dict) else None
    )
    model_name = (
        agent_config.get("model_name") if isinstance(agent_config, dict) else None
    )
    if not isinstance(model_name, str) or not model_name:
        model_name = agent_info.get("model_name")
    if not isinstance(model_name, str) or not model_name:
        model_name = None
    verifier = trial.get("verifier_result")
    rewards = verifier.get("rewards") if isinstance(verifier, dict) else None
    if not isinstance(rewards, dict):
        rewards = None
    exception = trial.get("exception_info")
    # The verifier decides whether the task was solved. A non-zero harness exit
    # after a correct answer is marked ``clean=False`` and shown separately,
    # but it does not erase a verifier reward.
    valid = rewards is not None and bool(rewards)
    clean = exception is None
    setup = trial.get("agent_setup") or {}
    execution = trial.get("agent_execution") or {}
    verifier_timing = trial.get("verifier") or {}

    trajectory = summarize_trajectory(trial_dir / "agent/trajectory.json")
    tool_usage = summarize_tool_usage(trial_dir, trajectory)
    try:
        trace_summary = summarize_trace(trial_dir, harness)
    except Exception as error:  # a broken native log must not hide the cell
        trace_summary = {"error": str(error)}
    artifacts = read_artifacts(trial_dir)
    run_ids = forward_run_ids(config)
    egress_run_id = run_ids.get(harness)
    egress_rows = egress.get(egress_run_id) if egress_run_id else None

    harbor_prompt = token_int(agent_context.get("n_input_tokens"))
    harbor_cached = token_int(agent_context.get("n_cache_tokens"))
    harbor_completion = token_int(agent_context.get("n_output_tokens"))
    harbor_cost = agent_context.get("cost_usd")
    if not isinstance(harbor_cost, (int, float)) or isinstance(harbor_cost, bool):
        harbor_cost = None

    cell = Cell(
        task=task,
        harness=harness,
        model=model_name,
        trial=trial_name,
        job_name=job_name,
        job_dir=job_dir,
        valid=valid,
        clean=clean,
        reward=rewards,
        exception=exception_summary(exception),
        agent_context=agent_context,
        agent_info=agent_info,
        started_at=trial.get("started_at"),
        finished_at=trial.get("finished_at"),
        agent_setup_sec=seconds_between(
            setup.get("started_at"), setup.get("finished_at")
        ),
        agent_execution_sec=seconds_between(
            execution.get("started_at"), execution.get("finished_at")
        ),
        verifier_sec=seconds_between(
            verifier_timing.get("started_at"), verifier_timing.get("finished_at")
        ),
        token_source=token_source,
        token_trusted=False,
        egress_run_id=egress_run_id,
        egress=egress_rows,
        harbor_prompt=harbor_prompt,
        harbor_cached=harbor_cached,
        harbor_completion=harbor_completion,
        harbor_cost=float(harbor_cost) if harbor_cost is not None else None,
        trajectory=trajectory,
        tool_usage=tool_usage,
        trace=trace_summary,
        artifacts=artifacts,
    )

    if token_source == "litecode":
        cell.token_source = "litecode 自报（agent_result ↔ litecode.txt）"
        usage = None
        try:
            usage = litecode_usage_line(
                (trial_dir / "agent/litecode.txt").read_text(
                    encoding="utf-8", errors="replace"
                )
            )
        except OSError:
            pass
        cell.litecode_line = usage
        if (
            harbor_prompt is not None
            and harbor_cached is not None
            and harbor_completion is not None
        ):
            cell.prompt, cell.cached, cell.completion = (
                harbor_prompt,
                harbor_cached,
                harbor_completion,
            )
            cell.token_trusted = True
            if usage is not None:
                line_prompt, line_completion, line_cached, _line_miss = usage
                if (line_prompt, line_cached, line_completion) != (
                    cell.prompt,
                    cell.cached,
                    cell.completion,
                ):
                    cell.token_trusted = False
                    cell.token_note = (
                        "litecode.txt 与 agent_result 不一致："
                        f"line=({line_prompt},{line_cached},{line_completion})"
                        f" result=({cell.prompt},{cell.cached},{cell.completion})"
                    )
                else:
                    cell.token_note = "agent_result 与 litecode.txt 一致"
            else:
                cell.token_trusted = False
                cell.token_note = "未找到 litecode.txt 的 usage 行"
        elif usage is not None:
            line_prompt, line_completion, line_cached, _line_miss = usage
            cell.prompt, cell.cached, cell.completion = (
                line_prompt,
                line_cached,
                line_completion,
            )
            cell.token_trusted = True
            cell.token_note = "agent_result 缺失，回退到 litecode.txt"
        else:
            cell.token_note = "litecode 自报 token 缺失"
        cell.requests = litecode_requests(trial_dir, trajectory)
    else:
        cell.token_source = "egress proxy（usage=ok 行）"
        if egress_rows is not None and egress_rows.requests:
            cell.prompt = egress_rows.prompt
            cell.cached = egress_rows.cached
            cell.completion = egress_rows.completion
            cell.requests = egress_rows.requests
            cell.aborted_requests = egress_rows.aborted
            cell.http_error_requests = egress_rows.http_errors
            cell.token_trusted = egress_rows.token_complete
            if egress_rows.successful_missing:
                cell.token_note = (
                    f"egress 有 {egress_rows.successful_missing} 个成功生成没有 usage，"
                    "token 与成本不可排名"
                )
            elif egress_rows.missing_usage:
                cell.token_note = (
                    f"egress 有 {egress_rows.missing_usage} 个失败/中断请求没有 usage，"
                    "已单列，不计费；ok 行完整"
                )
            else:
                cell.token_note = "egress 全部生成请求都有 usage"
            if (
                egress_rows.prompt != (harbor_prompt or 0)
                or egress_rows.cached != (harbor_cached or 0)
                or egress_rows.completion != (harbor_completion or 0)
            ):
                delta = (
                    f"harbor=({harbor_prompt},{harbor_cached},{harbor_completion})"
                )
                cell.token_note += (
                    "; 与 harness 自报不一致（以 egress 为准）：" + delta
                )
        else:
            cell.token_note = (
                "没有任何 egress 生成请求"
                + (f"（run_id {egress_run_id}）" if egress_run_id else "（未配置代理）")
            )

    cell.cache_trusted = cache_trusted_model(cell.model)
    if cell.cache_trusted:
        cell.cost = cost_of(cell.prompt, cell.cached, cell.completion)
        cell.cost_breakdown = cost_parts(
            cell.prompt, cell.cached, cell.completion
        )
    else:
        # The provider-reported cache split is a moving number; price the whole
        # prompt at the uncached rate instead. Keep the cache-based figure next
        # to it, clearly labelled as provider-reported.
        cell.reported_cost = cost_of(cell.prompt, cell.cached, cell.completion)
        cell.cost = cost_of(cell.prompt, 0, cell.completion)
        cell.cost_breakdown = cost_parts(cell.prompt, 0, cell.completion)
        note = (
            "非官方 provider 的 cache 命中不可信；成本按 prompt 全未命中计算"
        )
        if cell.reported_cost is not None:
            note += f"（provider 自报 cache 口径 ${cell.reported_cost:.6f}）"
        cell.token_note = (
            cell.token_note + "；" if cell.token_note else ""
        ) + note
    if cell.valid and not cell.passed:
        cell.failure_hint = read_failure_hint(trial_dir)

    # Strict rankability is deliberately separate from pass/fail. A verifier
    # result can be real while the billing row or trajectory is missing; such
    # a cell is shown but must not enter a cost/token ranking.
    policy = strict or {
        "require_token_complete": True,
        "require_trajectory": True,
        "require_clean_exit": False,
    }
    reasons: list[str] = []
    if not cell.valid:
        reasons.append("没有 verifier 结果")
    if policy.get("require_trajectory", True) and not cell.trajectory.get("present"):
        reasons.append("ATIF trajectory 缺失")
    if policy.get("require_token_complete", True) and not cell.token_trusted:
        reasons.append("token 不完整或不可信")
    if cell.requests is None or cell.requests <= 0:
        reasons.append("请求数缺失")
    if policy.get("require_clean_exit", False) and not cell.clean:
        reasons.append("harness 非零退出")
    warnings: list[str] = []
    if cell.http_error_requests:
        warnings.append(f"{cell.http_error_requests} 次上游 4xx/5xx")
    if cell.aborted_requests:
        warnings.append(f"{cell.aborted_requests} 次中断请求未产出 usage")
    cell.audit_reasons = reasons + warnings
    cell.rankable = not reasons
    return cell


def load_attempts(
    jobs_root: Path,
    egress: dict[str, EgressRows],
    campaign: Campaign | None = None,
) -> list[tuple[float, dict[str, Any], Path, Cell]]:
    allowed_tasks = set(campaign.task_names) if campaign else set(TASKS)
    allowed_harnesses = set(campaign.harness_ids) if campaign else set(HARNESSES)
    strict = campaign.strict if campaign else None
    attempts: list[tuple[float, dict[str, Any], Path, Cell]] = []
    for job_result in sorted(jobs_root.rglob("result.json")):
        job_dir = job_result.parent
        config = read_json(job_dir / "config.json")
        if not config:
            continue
        job_name = config.get("job_name")
        if not isinstance(job_name, str):
            continue
        # Harbor 0.25.0's job result is a stats rollup, not a trial list. Read
        # each <job>/<trial>/result.json; skip anything that is not one level
        # below the job directory.
        trial_results = sorted(job_dir.glob("*/result.json"))
        for trial_result in trial_results:
            trial = read_json(trial_result)
            if not trial:
                continue
            harness = harness_from_name((trial.get("agent_info") or {}).get("name"))
            task = task_from_name(trial.get("task_name"))
            if harness not in allowed_harnesses or task not in allowed_tasks:
                continue
            spec = campaign.harness_map.get(harness) if campaign else None
            token_source = (
                spec.token_source
                if spec is not None
                else ("litecode" if harness == "litecode" else "egress")
            )
            cell = build_cell(
                task=task,
                harness=harness,
                trial=trial,
                job_name=job_name,
                job_dir=job_dir,
                config=config,
                egress=egress,
                strict=strict,
                token_source=token_source,
            )
            if cell is not None:
                try:
                    mtime = trial_result.stat().st_mtime
                except OSError:
                    mtime = 0.0
                attempts.append((mtime, config, job_dir, cell))
    return attempts

def select_cells(
    attempts: list[tuple[float, dict[str, Any], Path, Cell]]
) -> dict[str, Cell]:
    selected: dict[str, Cell] = {}
    best_score: dict[str, tuple[int, int, int, float]] = {}
    for mtime, _config, _job_dir, cell in attempts:
        key = f"{cell.task}/{cell.harness}"
        # Rankable beats non-rankable; a verifier result beats no verifier
        # result; a clean run beats a verifier-passing non-zero exit; newest
        # wins only after data integrity has been considered.
        score = (
            1 if cell.rankable else 0,
            1 if cell.valid else 0,
            1 if cell.clean else 0,
            mtime,
        )
        if key not in best_score or score > best_score[key]:
            best_score[key] = score
            selected[key] = cell
    return selected

@dataclass
class Summary:
    harness: str
    cells: list[Cell]

    @property
    def valid_cells(self) -> list[Cell]:
        return [cell for cell in self.cells if cell.valid]

    @property
    def rankable_cells(self) -> list[Cell]:
        return [cell for cell in self.cells if cell.rankable]


@dataclass
class Totals:
    prompt: int | None = None
    cached: int | None = None
    completion: int | None = None
    cost: float | None = None
    requests: int | None = None
    agent_sec: float | None = None
    cells: int = 0
    passed: int = 0

    @classmethod
    def of(cls, cells: list[Cell]) -> "Totals":
        result = cls(cells=len(cells))
        prompt_values = [cell.prompt for cell in cells]
        cached_values = [cell.cached for cell in cells]
        completion_values = [cell.completion for cell in cells]
        cost_values = [cell.cost for cell in cells]
        request_values = [cell.requests for cell in cells]
        time_values = [cell.agent_execution_sec for cell in cells]
        result.passed = sum(1 for cell in cells if cell.passed)
        if all(value is not None for value in prompt_values):
            result.prompt = sum(int(value) for value in prompt_values)
        if all(value is not None for value in cached_values):
            result.cached = sum(int(value) for value in cached_values)
        if all(value is not None for value in completion_values):
            result.completion = sum(int(value) for value in completion_values)
        if all(value is not None for value in cost_values):
            result.cost = sum(float(value) for value in cost_values)
        if all(value is not None for value in request_values):
            result.requests = sum(int(value) for value in request_values)
        if all(value is not None for value in time_values):
            result.agent_sec = sum(float(value) for value in time_values)
        return result

    @property
    def total_tokens(self) -> int | None:
        if self.prompt is None or self.completion is None:
            return None
        return self.prompt + self.completion

    @property
    def cache_rate(self) -> float | None:
        if not self.prompt or self.cached is None:
            return None
        return self.cached / self.prompt

    def avg_prompt_per_request(self) -> float | None:
        if not self.requests or self.prompt is None:
            return None
        return self.prompt / self.requests


def summary_for(harness: str, selected: dict[str, Cell]) -> Summary:
    cells = [
        selected[f"{task}/{harness}"]
        for task in TASKS
        if f"{task}/{harness}" in selected
    ]
    return Summary(harness=harness, cells=cells)


def md_table(headers: list[str], rows: list[list[str]]) -> list[str]:
    lines = [
        "| " + " | ".join(headers) + " |",
        "|" + "|".join("---" for _ in headers) + "|",
    ]
    for row in rows:
        lines.append("| " + " | ".join(row) + " |")
    return lines


def format_cell_result(cell: Cell | None) -> str:
    if cell is None:
        return "缺失"
    if not cell.valid:
        return "异常"
    if cell.passed:
        text = "通过" if cell.clean else "通过（非零退出）"
    else:
        text = "失败"
    return text if cell.rankable else text + "⚠"


def litecode_build_info(selected: dict[str, Cell]) -> dict[str, Any] | None:
    """The build that actually ran, read from the LiteCode trial artifacts.

    A checkout-local build-info says what could run, not what did run. When the
    selected trials disagree, report that explicitly instead of picking one.
    """
    infos: dict[str, dict[str, Any]] = {}
    for cell in selected.values():
        if cell.harness != "litecode" or not cell.trial:
            continue
        info = read_json(
            cell.job_dir / cell.trial / "agent/litecode/build-info.json"
        )
        if info:
            infos[json.dumps(info, sort_keys=True)] = info
    if len(infos) == 1:
        return next(iter(infos.values()))
    if len(infos) > 1:
        return {"commit": f"{len(infos)} different builds", "branch": "", "dirty": "?", "built_at": "?"}
    return None


def build_report(
    campaign: Campaign,
    out_md: Path,
    out_json: Path,
    repo: Path,
    jobs_root: Path | None = None,
    egress_path: Path | None = None,
) -> int:
    global TASKS, HARNESSES
    global PRICE_UNCACHED_INPUT, PRICE_CACHED_INPUT, PRICE_OUTPUT

    TASKS = campaign.task_names
    HARNESSES = campaign.harness_ids
    PRICE_UNCACHED_INPUT = campaign.price["uncached_input"] / 1_000_000
    PRICE_CACHED_INPUT = campaign.price["cached_input"] / 1_000_000
    PRICE_OUTPUT = campaign.price["output"] / 1_000_000

    jobs_root = jobs_root or campaign.jobs_root
    egress_path = egress_path or campaign.egress_log
    egress = load_egress(egress_path)
    attempts = load_attempts(jobs_root, egress, campaign)
    selected = select_cells(attempts)
    summaries = {harness: summary_for(harness, selected) for harness in HARNESSES}
    totals = {
        harness: Totals.of(summary.rankable_cells)
        for harness, summary in summaries.items()
    }

    lines: list[str] = []
    litecode_models = sorted(
        {
            cell.model
            for cell in selected.values()
            if cell.harness == "litecode" and cell.model
        }
    )
    if litecode_models and any(model != campaign.model for model in litecode_models):
        model_note = (
            f"LiteCode 使用 `{', '.join(litecode_models)}`；"
            f"其余 harness 使用基线 `{campaign.model}`"
        )
    else:
        model_note = f"所有 harness 同一模型 `{campaign.model}`"
    lines.append(f"# {campaign.name} 横评（{campaign.model}）")
    lines.append("")
    task_text = "、".join(
        f"`{task.name}`（{task.difficulty}）" for task in campaign.tasks
    )
    lines.append(
        f"> 生成时间 `{datetime.now().astimezone().isoformat(timespec='seconds')}`。"
        f" {len(campaign.tasks)} 题：{task_text}。{model_note}，"
        "思考强度走默认 high（LiteCode 的 `medium` 按各自 catalog 映射）。"
    )
    lines.append("")
    lines.append("## 0. 数据来源与口径")
    lines.append("")
    lines.append(
        "* **执行/判题/轨迹**：Harbor 0.25.0；每题 1 trial，"
        f"{len(campaign.harnesses)} 家 harness 并发 {campaign.concurrency}；"
        "基础设施失败（缺少 trial 或 exception）不进入通过率分母，自动重跑。"
    )
    lines.append(
        "* **token 计费**：LiteCode 用 CLI 自报的 usage 行，"
        "其余 harness 只用 egress proxy 的逐请求 `usage=ok` 记录；"
        "harness 自报仅作对照，不作为分数。"
    )
    lines.append(
        "* **价格**：未命中输入 "
        f"`${campaign.price['uncached_input']}/M`、"
        f"命中缓存 `${campaign.price['cached_input']}/M`、"
        f"输出 `${campaign.price['output']}/M`；成本由 token 按此价目重算，"
        "LiteCode 同时保留 Harbor/LiteLLM 计价做核验。"
    )
    if any(not cell.cache_trusted for cell in selected.values()):
        lines.append(
            "* **缓存口径**：非官方 DeepSeek provider 的 cache 命中会抖动且不可信；"
            "这些 cell 的成本按 prompt 全未命中计算，provider 自报 cache 口径"
            "另见 JSON 的 `reported_cost_usd`。"
        )
    lines.append(
        "* **token 定义**：prompt 含缓存命中；output 含思考；"
        "total = prompt + output；cache hit = cached / prompt。"
    )
    lines.append("")

    build_info = litecode_build_info(selected)
    if build_info:
        lines.append(
            "* LiteCode build：commit `{}`，branch `{}`，dirty `{}`，"
            "built_at `{}`。".format(
                str(build_info.get("commit", "?"))[:12],
                build_info.get("branch"),
                build_info.get("dirty"),
                build_info.get("built_at"),
            )
        )
    lines.append("* 任务镜像（campaign manifest，公开 task images）：")
    for task in campaign.tasks:
        source = f"（source `{task.source_image}`）" if task.source_image and task.source_image != task.image else ""
        lines.append(f"  * `{task.name}`：`{task.image}`{source}")
    if campaign.harness_runtime:
        lines.append(
            "* 共享 harness runtime：`{}`，只读挂载到 `/opt/harness`。".format(
                campaign.harness_runtime
            )
        )
    lines.append("")

    lines.append("## 1. 通过矩阵")
    lines.append("")
    headers = [
        "harness",
        *TASKS,
        "pass@1",
        "有效格",
        "可排名格",
        "非零退出",
    ]
    rows = []
    for harness in HARNESSES:
        summary = summaries[harness]
        row = [f"`{harness}`"]
        for task in TASKS:
            row.append(format_cell_result(selected.get(f"{task}/{harness}")))
        valid = len(summary.valid_cells)
        rankable = summary.rankable_cells
        passed = sum(1 for cell in rankable if cell.passed)
        dirty = sum(1 for cell in rankable if not cell.clean)
        row.append(f"{passed}/{len(rankable)}" if rankable else "缺失")
        row.append(str(valid))
        row.append(str(len(rankable)))
        row.append(str(dirty))
        rows.append(row)
    lines.extend(md_table(headers, rows))
    lines.append("")

    # Failure diagnosis: the verifier is the arbiter, so quote its assertion
    # rather than guessing from a trajectory.
    lines.append("## 1b. 失败诊断")
    lines.append("")
    failed_cells = [
        (task, harness, selected[f"{task}/{harness}"])
        for task in TASKS
        for harness in HARNESSES
        if f"{task}/{harness}" in selected
        and selected[f"{task}/{harness}"].valid
        and not selected[f"{task}/{harness}"].passed
    ]
    if failed_cells:
        for task, harness, cell in failed_cells:
            hint = cell.failure_hint or "verifier 未给出可摘录的失败行"
            lines.append(f"* `{harness}` / `{task}`：{hint}")
    else:
        lines.append("* 没有 verifier 判定的失败格。")
    lines.append("")
    lines.append("## 2. 逐格关键指标")
    lines.append("")
    headers = [
        "task",
        "harness",
        "结果",
        "可排名",
        "请求数",
        "prompt",
        "cache 命中",
        "output",
        "total token",
        "成本",
        "agent 时间",
        "轨迹步骤/工具",
    ]
    rows = []
    for task in TASKS:
        for harness in HARNESSES:
            cell = selected.get(f"{task}/{harness}")
            if cell is None:
                rows.append(
                    [task, f"`{harness}`", "缺失", "缺失", "缺失", "缺失",
                     "缺失", "缺失", "缺失", "缺失", "缺失", "缺失"]
                )
                continue
            trajectory = cell.trajectory
            trace_text = "缺失"
            if trajectory.get("present"):
                trace_text = (
                    f"{trajectory.get('n_steps', 0)} 步 / "
                    f"{trajectory.get('n_tool_calls', 0)} 工具"
                )
            rows.append(
                [
                    task,
                    f"`{harness}`",
                    format_cell_result(cell),
                    "是" if cell.rankable else "否",
                    num(cell.requests),
                    k(cell.prompt),
                    pct(cell.cache_rate),
                    k(cell.completion),
                    k(cell.total_tokens),
                    money(cell.cost),
                    f"{cell.agent_execution_sec:.1f}s"
                    if cell.agent_execution_sec is not None
                    else "缺失",
                    trace_text,
                ]
            )
    lines.extend(md_table(headers, rows))
    lines.append("")

    lines.append(
        f"## 3. 汇总（{len(TASKS)} 题合计，仅可排名 trial）"
    )
    lines.append("")
    headers = [
        "harness",
        "通过",
        "可排名格",
        "非零退出",
        "请求/题",
        "prompt/题",
        "prompt/请求",
        "cache 命中",
        "output/题",
        "total token/题",
        "成本/题",
        "成本/通过题",
        "agent 时间/题",
    ]
    rows = []
    for harness in HARNESSES:
        summary = summaries[harness]
        total = totals[harness]
        cells = summary.rankable_cells
        if not cells:
            rows.append([f"`{harness}`"] + ["缺失"] * 12)
            continue
        rows.append(
            [
                f"`{harness}`",
                f"{total.passed}/{len(cells)}",
                str(len(cells)),
                str(sum(1 for cell in cells if not cell.clean)),
                f"{total.requests / len(cells):.1f}"
                if total.requests is not None
                else "缺失",
                k(total.prompt / len(cells)) if total.prompt is not None else "缺失",
                k(total.avg_prompt_per_request())
                if total.avg_prompt_per_request() is not None
                else "缺失",
                pct(total.cache_rate),
                k(total.completion / len(cells))
                if total.completion is not None
                else "缺失",
                k(total.total_tokens / len(cells))
                if total.total_tokens is not None
                else "缺失",
                money(total.cost / len(cells))
                if total.cost is not None
                else "缺失",
                money(total.cost / total.passed)
                if total.cost is not None and total.passed
                else "缺失",
                f"{total.agent_sec / len(cells):.1f}s"
                if total.agent_sec is not None
                else "缺失",
            ]
        )
    lines.extend(md_table(headers, rows))
    lines.append("")

    lines.append("## 4. LiteCode 与其他 harness 的差距")
    lines.append("")
    lite = summaries["litecode"]
    lite_total = totals["litecode"]
    other_totals = {name: totals[name] for name in HARNESSES if name != "litecode"}

    best_other_pass = max((total.passed for total in other_totals.values()), default=None)
    passed_other = [
        name for name, total in other_totals.items() if total.passed == best_other_pass
    ]
    if lite.rankable_cells and passed_other:
        best_summary = summaries[passed_other[0]]
        if best_summary.rankable_cells:
            lite_rate = lite_total.passed / len(lite.rankable_cells)
            best_rate = best_other_pass / len(best_summary.rankable_cells)
            lines.append(
                f"* **通过率**：LiteCode {lite_total.passed}/{len(lite.rankable_cells)}"
                f"（{pct(lite_rate)}）；其他 harness 最高 {best_other_pass} 格"
                f"（{pct(best_rate)}，{', '.join(passed_other)}）。"
                f"差值 {((lite_rate or 0) - (best_rate or 0)) * 100:+.1f} 个百分点。"
            )
        else:
            lines.append(
                "* **通过率**：本次没有其他 harness 的可排名基线，无法计算差值。"
            )

    comparable = [
        (name, total)
        for name, total in other_totals.items()
        if total.cost is not None and total.cells == len(lite.rankable_cells)
    ]
    if comparable and lite_total.cost is not None and lite.rankable_cells:
        cheapest_name, cheapest = min(comparable, key=lambda item: item[1].cost or 0)
        cost_gap = lite_total.cost - (cheapest.cost or 0)
        ratio = lite_total.cost / cheapest.cost if cheapest.cost else None
        lines.append(
            f"* **成本**：LiteCode {len(TASKS)} 题合计 {money(lite_total.cost)}；"
            f"其他 harness 最低 {cheapest_name} {money(cheapest.cost)}；"
            f"差距 {money(cost_gap)}（{ratio:.2f}×）。"
        )
        lite_parts = {
            key: sum(float(cell.cost_breakdown.get(key) or 0) for cell in lite.rankable_cells)
            for key in ("uncached", "cached", "output")
        }
        cheap_parts = {
            key: sum(float(cell.cost_breakdown.get(key) or 0) for cell in summaries[cheapest_name].rankable_cells)
            for key in ("uncached", "cached", "output")
        }
        lines.append(
            "  - 成本差拆项：未命中输入 "
            f"{money(lite_parts['uncached'] - cheap_parts['uncached'])}，"
            f"命中缓存 {money(lite_parts['cached'] - cheap_parts['cached'])}，"
            f"输出 {money(lite_parts['output'] - cheap_parts['output'])}。"
        )

    lean_candidates = [
        (name, total)
        for name, total in other_totals.items()
        if total.total_tokens is not None and total.cells == len(lite.rankable_cells)
    ]
    if lean_candidates and lite_total.total_tokens is not None and lite.rankable_cells:
        lean_name, lean = min(lean_candidates, key=lambda item: item[1].total_tokens or 0)
        ratio = lite_total.total_tokens / lean.total_tokens if lean.total_tokens else None
        lines.append(
            f"* **token 效率**：LiteCode {len(TASKS)} 题合计 {num(lite_total.total_tokens)} "
            f"token；其他 harness 最低 {lean_name} {num(lean.total_tokens)}；"
            f"LiteCode 是它的 {ratio:.2f}×。"
            if ratio is not None
            else "* **token 效率**：token 数据不完整。"
        )
        if (
            lite_total.requests
            and lean.requests
            and lite_total.prompt is not None
            and lean.prompt is not None
        ):
            lite_ctx = lite_total.prompt / lite_total.requests
            lean_ctx = lean.prompt / lean.requests
            req_gap = lite_total.requests - lean.requests
            turn_effect = req_gap * lean_ctx
            context_effect = lite_total.requests * (lite_ctx - lean_ctx)
            interaction = req_gap * (lite_ctx - lean_ctx)
            prompt_gap = lite_total.prompt - lean.prompt
            lines.append(
                f"* **差距在哪里**（LiteCode vs {lean_name}）："
                f"LiteCode {lite_total.requests} 次请求 × 平均 {k(lite_ctx)} prompt/请求；"
                f"{lean_name} {lean.requests} 次 × 平均 {k(lean_ctx)}。"
                f"prompt 总差 {num(prompt_gap)} token，其中"
                f"轮次多出 {req_gap:+d} 次贡献 {num(round(turn_effect))} token，"
                f"每请求上下文贡献 {num(round(context_effect))} token，"
                f"交互项 {num(round(interaction))} token。"
            )
            if (
                lite_total.completion is not None
                and lean.completion is not None
            ):
                lines.append(
                    f"* **输出侧**：LiteCode 输出 {num(lite_total.completion)} token，"
                    f"{lean_name} {num(lean.completion)}；"
                    f"差值 {lite_total.completion - lean.completion:+,} token。"
                )
    lines.append("")
    lines.append("## 5. 逐家观察")
    lines.append("")
    for harness in HARNESSES:
        summary = summaries[harness]
        total = totals[harness]
        cells = summary.rankable_cells
        if not cells:
            lines.append(f"* **{harness}**：没有合法 trial，不能进入结论。")
            continue
        parts = [
            f"{total.passed}/{len(cells)} 通过",
            f"{total.requests / len(cells):.1f} 请求/题"
            if total.requests is not None
            else "请求数缺失",
            f"prompt/题 {k(total.prompt / len(cells))}"
            if total.prompt is not None
            else "prompt 缺失",
            f"prompt/请求 {k(total.avg_prompt_per_request())}"
            if total.avg_prompt_per_request() is not None
            else "prompt/请求 缺失",
            f"cache {pct(total.cache_rate)}",
            f"output/题 {k(total.completion / len(cells))}"
            if total.completion is not None
            else "output 缺失",
            f"成本/题 {money(total.cost / len(cells))}"
            if total.cost is not None
            else "成本缺失",
        ]
        notes = []
        exceptions = [
            f"{cell.task}: {exception_label(cell.exception)}"
            for cell in cells
            if not cell.clean
        ]
        if exceptions:
            notes.append("非零退出：" + "；".join(exceptions))
        for cell in cells:
            if cell.token_note and cell.token_note not in notes:
                notes.append(cell.token_note)
        lines.append(
            f"* **{harness}**：" + "；".join(parts) + "。"
            + ((" 记账核对：" + "；".join(notes) + "。") if notes else "")
        )
    lines.append("")
    lines.append("## 5b. LiteCode 工具接口观测")
    lines.append("")
    lines.append("| task | run_code | nested calls | top nested tools | reminders |")
    lines.append("|---|---:|---:|---|---:|")
    for task in TASKS:
        cell = selected.get(f"{task}/litecode")
        if cell is None:
            lines.append(f"| {task} | missing | missing | missing | missing |")
            continue
        usage = cell.tool_usage or {}
        audit = usage.get("audit") or {}
        calls = usage.get("calls_by_tool") or {}
        if audit.get("present"):
            run_calls = int(calls.get("run_code") or 0)
            nested = str(sum(v for key, v in calls.items() if key != "run_code"))
            top = ", ".join(
                f"{key}:{value}"
                for key, value in sorted(
                    calls.items(), key=lambda item: (-item[1], item[0])
                )
                if key != "run_code"
            ) or "-"
        else:
            run_calls = cell.trajectory.get("tool_calls", {}).get("run_code", 0)
            nested = "n/a"
            top = "n/a"
        reminders = usage.get("reminders") or {}
        lines.append(
            f"| {task} | {run_calls} | {nested} | {top} | {reminders.get('lines', 0)} |"
        )
    lines.append("")

    lines.append("## 6. 不可比项与口径边界")
    lines.append("")
    lines.append(
        f"1. **只有 {len(TASKS)} 题**，pass@1 的统计意义有限；结论主要是工程效率与成本，"
        "不是模型能力排名。"
    )
    lines.append(
        "2. **LiteCode 不走代理**：它按编译进二进制的官方 DeepSeek 端点直连，"
        "token 来自 CLI 最终 usage 行；其余走代理的 harness 共用同一个 egress。"
        "因此 LiteCode 的 token 是自报口径（已与 trial artifact 对照），"
        "不是代理逐请求口径。"
    )
    lines.append(
        "3. **opencode 自报会漏思考输出**：本报告不用它的自报 token；"
        "凡是以 egress 为准的格，都在 JSON 里保留了 harness 自报值与差异。"
    )
    unclean = sum(
        1
        for cell in selected.values()
        if cell.harness == "opencode" and cell.valid and not cell.clean
    )
    lines.append(
        "3b. **opencode 的进程状态**：适配器把 JSON 流中的 "
        "`step_finish.stop` 视为成功，不因流结束后的传输噪声判失败；"
        f"本次 opencode 非零退出格数为 {unclean}。"
    )
    lines.append(
        "4. **step 定义不可横比**：各 harness 的 ATIF 轨迹由各自 adapter 生成，"
        "步/工具数的定义不同；表里只作诊断线索，不参与排名。"
    )
    lines.append(
        "5. **成本价目**：LiteCode 自报的 Harbor/LiteLLM 成本只在 JSON 中作核对；"
        "主表统一按上文三项单价重算，避免各 harness 计价表不一致。"
    )
    lines.append(
        "6. **异常的处理**：没有 verifier 结果的格写 '异常/缺失'，不计入"
        "通过率；verifier 已判通过、但 harness 进程非零退出的格写"
        "'通过（非零退出）'，通过照算，同时单列提醒这是 harness 适配问题。"
    )
    lines.append("")
    lines.append(
        "7. **polyglot-c-py 的清理陷阱**：instruction 自己给出了 "
        "`gcc ... -o /app/polyglot/cmain`，但 verifier 要求该目录最终只剩 "
        "`main.py.c`。本次 LiteCode 和 opencode 都通过，但这个约束仍是"
        "最终状态纪律/对 verifier 的推断，而不是 polyglot 代码写不出来。"
    )
    lines.append("")
    report_text = "\n".join(lines) + "\n"
    out_md.parent.mkdir(parents=True, exist_ok=True)
    out_md.write_text(report_text, encoding="utf-8")

    expected_cells = campaign.expected_cells
    missing_cells = sorted(
        key for key in expected_cells if key not in selected
    )
    unrankable_cells = sorted(
        key
        for key in expected_cells
        if key in selected and not selected[key].rankable
    )
    payload = {
        "generated_at": datetime.now().astimezone().isoformat(timespec="seconds"),
        "campaign": {
            "name": campaign.name,
            "path": str(campaign.path),
            "model": campaign.model,
            "litecode_models": litecode_models,
            "strict": campaign.strict,
        },
        "tasks": list(TASKS),
        "harnesses": list(HARNESSES),
        "expected_cells": sorted(expected_cells),
        "unrankable_cells": unrankable_cells,
        "price_usd_per_million": {
            "uncached_input": PRICE_UNCACHED_INPUT * 1_000_000,
            "cached_input": PRICE_CACHED_INPUT * 1_000_000,
            "output": PRICE_OUTPUT * 1_000_000,
        },
        "cells": [selected[key].to_json() for key in sorted(selected)],
        "missing_cells": missing_cells,
        "summary": {
            harness: {
                "passed": totals[harness].passed,
                "valid_cells": len(summaries[harness].valid_cells),
                "rankable_cells": len(summaries[harness].rankable_cells),
                "exceptions": sum(
                    1
                    for cell in summaries[harness].rankable_cells
                    if not cell.clean
                ),
                "requests": totals[harness].requests,
                "prompt": totals[harness].prompt,
                "cached": totals[harness].cached,
                "completion": totals[harness].completion,
                "total_tokens": totals[harness].total_tokens,
                "cost_usd": totals[harness].cost,
                "agent_sec": totals[harness].agent_sec,
            }
            for harness in HARNESSES
        },
        "egress_rows": sum(len(bucket.rows) for bucket in egress.values()),
    }
    payload["tool_usage"] = [
        {
            "task": selected[key].task,
            "harness": selected[key].harness,
            **selected[key].tool_usage,
        }
        for key in sorted(selected)
        if selected[key].harness == "litecode" and selected[key].tool_usage
    ]
    payload["traces"] = [
        {
            "task": selected[key].task,
            "harness": selected[key].harness,
            **selected[key].trace,
        }
        for key in sorted(selected)
        if selected[key].trace
    ]
    selected_export = {
        key: {
            "task": selected[key].task,
            "harness": selected[key].harness,
            "trial": selected[key].trial,
            "job_name": selected[key].job_name,
            "job_dir": str(selected[key].job_dir),
            "egress_run_id": selected[key].egress_run_id,
            "valid": selected[key].valid,
            "clean": selected[key].clean,
            "rankable": selected[key].rankable,
            "passed": selected[key].passed,
            "reward": selected[key].reward,
            "tokens": {
                "prompt_including_cache": selected[key].prompt,
                "cached": selected[key].cached,
                "output": selected[key].completion,
                "total": selected[key].total_tokens,
            },
            "requests": selected[key].requests,
            "tool_usage": selected[key].tool_usage,
            "trace": selected[key].trace,
            "cost_usd": selected[key].cost,
            "reported_cost_usd": selected[key].reported_cost,
            "cache_trusted": selected[key].cache_trusted,
            "agent_execution_sec": selected[key].agent_execution_sec,
            "audit_reasons": selected[key].audit_reasons,
        }
        for key in sorted(expected_cells)
        if key in selected
    }
    registry = jobs_root / "registry"
    registry.mkdir(parents=True, exist_ok=True)
    (registry / "selected.json").write_text(
        json.dumps(selected_export, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    payload["selected"] = selected_export
    out_json.write_text(
        json.dumps(payload, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    print(f"wrote {out_md}", flush=True)
    print(f"wrote {out_json}", flush=True)
    if missing_cells:
        print("missing cells: " + ", ".join(missing_cells), file=sys.stderr)
    if unrankable_cells:
        print("unrankable cells: " + ", ".join(unrankable_cells), file=sys.stderr)
    strict_fail = bool(unrankable_cells)
    if campaign.strict.get("require_expected_cells", True) and missing_cells:
        strict_fail = True
    return 1 if strict_fail else 0

def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    home = Path.home()
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument(
        "--campaign",
        default=str(DEFAULT_CAMPAIGN),
        help="campaign manifest: tasks, harnesses, paths, prices, strict policy",
    )
    parser.add_argument("--jobs-root", default=None)
    parser.add_argument("--egress", default=None)
    parser.add_argument("--repo", default=str(Path(__file__).resolve().parents[3]))
    parser.add_argument("--out-md", default=str(home / "eval/bench5-report.md"))
    parser.add_argument("--out-json", default=str(home / "eval/bench5-report.json"))
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        campaign = load_campaign(args.campaign)
        return build_report(
            campaign,
            Path(args.out_md).expanduser().resolve(),
            Path(args.out_json).expanduser().resolve(),
            Path(args.repo).expanduser().resolve(),
            jobs_root=(
                Path(args.jobs_root).expanduser().resolve()
                if args.jobs_root
                else None
            ),
            egress_path=(
                Path(args.egress).expanduser().resolve() if args.egress else None
            ),
        )
    except (OSError, ValueError, KeyError, CampaignError) as error:
        print(f"collect: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())