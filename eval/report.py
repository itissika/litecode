#!/usr/bin/env python3
"""One report from two sources: Harbor's own numbers and the proxy's rows.

Harbor writes a job result (<job>/result.json) with stats.* totals and one
agent_result.* per trial. The egress proxy writes one JSONL row per upstream
request. Neither is trusted alone: the report puts them side by side and flags
every place they disagree, instead of picking the prettier one.

Rules (STATE.md section 2):

- token counts come from stats (job) and agent_result (trial) only - never
  from a top-level key that happens to exist;
- n_input_tokens includes cached tokens;
- a number that is absent stays absent: it is printed as 缺失 and flagged, not
  filled with zero and not estimated.

Usage:

    python eval/report.py --job-dir results/harbor/ds3-r1 \
        --egress ~/eval/logs/egress.jsonl [--detail] [--out report.md]

--job-dir may be repeated; a directory must hold result.json.
"""

from __future__ import annotations

import argparse
import json
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

MISSING = "缺失"
DASH = "—"


@dataclass
class Sums:
    """Token totals, plus how much of them was actually reported."""

    prompt: int = 0
    completion: int = 0
    cached: int = 0
    cost: float = 0.0
    cost_seen: bool = False
    missing: int = 0
    trials: int = 0

    def take_context(self, context: Any) -> None:
        self.trials += 1
        if not isinstance(context, dict):
            self.missing += 3
            return
        self._take(context, "n_input_tokens", "prompt")
        self._take(context, "n_output_tokens", "completion")
        self._take(context, "n_cache_tokens", "cached")
        cost = context.get("cost_usd")
        if isinstance(cost, (int, float)) and not isinstance(cost, bool):
            self.cost += float(cost)
            self.cost_seen = True

    def _take(self, source: dict[str, Any], key: str, attr: str) -> None:
        value = source.get(key)
        if isinstance(value, int) and not isinstance(value, bool):
            setattr(self, attr, getattr(self, attr) + value)
        else:
            self.missing += 1

    def take_stats(self, stats: dict[str, Any]) -> None:
        self._take(stats, "n_input_tokens", "prompt")
        self._take(stats, "n_output_tokens", "completion")
        self._take(stats, "n_cache_tokens", "cached")
        cost = stats.get("cost_usd")
        if isinstance(cost, (int, float)) and not isinstance(cost, bool):
            self.cost += float(cost)
            self.cost_seen = True


@dataclass
class Egress:
    """What the proxy saw for one run id, and what it could not see."""

    rows: int = 0
    missing: int = 0
    sums: Sums = field(default_factory=Sums)

    def take(self, row: dict[str, Any]) -> None:
        self.rows += 1
        state = row.get("usage")
        if state == "n/a":
            return
        if state != "ok":
            self.missing += 1
            return
        for key, attr in (
            ("prompt_tokens", "prompt"),
            ("completion_tokens", "completion"),
            ("cached_tokens", "cached"),
        ):
            value = row.get(key)
            if isinstance(value, int) and not isinstance(value, bool):
                setattr(self.sums, attr, getattr(self.sums, attr) + value)
            else:
                self.sums.missing += 1


def _int(value: Any) -> str:
    return MISSING if value is None else f"{value:,}"


def _signed(value: int) -> str:
    return f"+{value:,}" if value > 0 else f"{value:,}"


def _float(value: Any) -> str:
    return MISSING if value is None else f"${value:.4f}"


def load_job(path: Path) -> tuple[dict[str, Any], dict[str, Any]]:
    """The job result, plus the job config that declares its harnesses."""
    result = path / "result.json"
    if not result.is_file():
        raise FileNotFoundError(f"{path} has no result.json")
    document = json.loads(result.read_text(encoding="utf-8"))
    if not isinstance(document, dict):
        raise ValueError(f"{result} is not a job result object")
    config_path = path / "config.json"
    config: dict[str, Any] = {}
    if config_path.is_file():
        loaded = json.loads(config_path.read_text(encoding="utf-8"))
        if isinstance(loaded, dict):
            config = loaded
    return document, config


def declared_harnesses(config: dict[str, Any]) -> list[str]:
    names = []
    for agent in config.get("agents") or []:
        if isinstance(agent, dict) and isinstance(agent.get("name"), str):
            names.append(agent["name"])
    return names


def trial_records(job: dict[str, Any]) -> list[dict[str, Any]]:
    records = []
    for trial in job.get("trial_results") or []:
        info = trial.get("agent_info") or {}
        verifier = trial.get("verifier_result") or {}
        exception = trial.get("exception_info") or {}
        records.append(
            {
                "task": trial.get("task_name") or trial.get("trial_name"),
                "trial": trial.get("trial_name"),
                "agent": info.get("name"),
                "context": trial.get("agent_result"),
                "reward": verifier.get("rewards"),
                "error": exception.get("exception_type"),
            }
        )
    return records


def load_egress(path: Path) -> list[dict[str, Any]]:
    rows = []
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip():
            continue
        try:
            row = json.loads(line)
        except ValueError as error:
            raise ValueError(f"{path}:{number}: not JSON: {error}") from error
        if isinstance(row, dict):
            rows.append(row)
    return rows


def egress_for(rows: list[dict[str, Any]], job: str, harness: str) -> Egress:
    """Rows belonging to one harness's run id.

    The resolver names every run "<job>-<harness>"; a single-agent job that
    predates that convention may still use the bare job name.
    """
    suffix = f"-{harness}"
    chosen = []
    for row in rows:
        run_id = row.get("run_id")
        if not isinstance(run_id, str):
            continue
        if run_id == job or (
            run_id.startswith(f"{job}-") and run_id.endswith(suffix)
        ):
            chosen.append(row)
    bucket = Egress()
    for row in chosen:
        bucket.take(row)
    return bucket


def flag_diff(label: str, ours: int, theirs: int) -> str | None:
    if ours == theirs:
        return None
    return (
        f"⚠ {label} 差 {_signed(ours - theirs)}"
        f"（Harbor {_int(ours)} / egress {_int(theirs)}）"
    )


def _reward_text(reward: Any) -> str:
    if not isinstance(reward, dict) or not reward:
        return DASH
    return ", ".join(f"{key}={value}" for key, value in reward.items())


def render_job(
    name: str,
    job: dict[str, Any],
    config: dict[str, Any],
    egress_rows: list[dict[str, Any]] | None,
    *,
    detail: bool,
) -> list[str]:
    stats = job.get("stats") or {}
    records = trial_records(job)
    lines = [f"## Job {name}", ""]

    job_sums = Sums()
    job_sums.take_stats(stats)
    lines.append(
        "job stats: prompt "
        f"{_int(stats.get('n_input_tokens'))}（含 cache）"
        f" · completion {_int(stats.get('n_output_tokens'))}"
        f" · cached {_int(stats.get('n_cache_tokens'))}"
        f" · cost {_float(stats.get('cost_usd'))}"
        f" · trials {_int(stats.get('n_completed_trials'))} 完成"
        f" / {_int(stats.get('n_errored_trials'))} 失败"
    )

    flags: list[str] = []
    for key, label in (
        ("n_input_tokens", "prompt"),
        ("n_output_tokens", "completion"),
        ("n_cache_tokens", "cached"),
        ("cost_usd", "cost"),
    ):
        if stats.get(key) is None:
            flags.append(f"⚠ job stats 缺 {label}（顶层没有这四个键）")

    harnesses: dict[str, list[dict[str, Any]]] = {
        harness: [] for harness in declared_harnesses(config)
    }
    for record in records:
        harnesses.setdefault(record["agent"] or "?", []).append(record)
    declared = set(declared_harnesses(config))

    lines += [
        "",
        "| harness | trials | prompt（含 cache） | completion | cached | cost |"
        " egress prompt | egress completion | egress cached | 判定 |",
        "|---|---|---|---|---|---|---|---|---|---|",
    ]
    total_egress = Sums()
    for harness, items in harnesses.items():
        sums = Sums()
        errored = 0
        for item in items:
            sums.take_context(item["context"])
            if item["error"]:
                errored += 1
        judge: list[str] = []
        if harness in declared and not items:
            judge.append("⚠ job 配了这一家，但没有任何 trial 记录")
        if sums.missing:
            judge.append(f"⚠ {sums.missing} 个 token 字段缺失")
        if errored:
            judge.append(f"⚠ {errored} 个 trial 抛错")
        if egress_rows is None:
            egress_cells = [DASH, DASH, DASH]
        else:
            bucket = egress_for(egress_rows, name, harness)
            egress_cells = [
                _int(bucket.sums.prompt),
                _int(bucket.sums.completion),
                _int(bucket.sums.cached),
            ]
            total_egress.prompt += bucket.sums.prompt
            total_egress.completion += bucket.sums.completion
            total_egress.cached += bucket.sums.cached
            if bucket.rows == 0:
                judge.append(f"⚠ 无 egress 行（run_id {name}-{harness}）")
            else:
                for label, ours, theirs in (
                    ("prompt", sums.prompt, bucket.sums.prompt),
                    ("completion", sums.completion, bucket.sums.completion),
                    ("cached", sums.cached, bucket.sums.cached),
                ):
                    note = flag_diff(label, ours, theirs)
                    if note:
                        judge.append(note)
                if bucket.missing:
                    judge.append(f"⚠ egress 有 {bucket.missing} 行没拿到 usage")
        prompt_cell = _int(sums.prompt)
        if sums.missing:
            prompt_cell += "（不完整）"
        completion_cell = _int(sums.completion)
        cached_cell = _int(sums.cached)
        cost_cell = _float(sums.cost if sums.cost_seen else None)
        if not items:
            # A harness the job declared but no trial ever reached has no
            # numbers at all; zeros would read as "it ran and spent nothing".
            prompt_cell = completion_cell = cached_cell = MISSING
            cost_cell = MISSING
        lines.append(
            f"| {harness} | {len(items)} | {prompt_cell} | "
            f"{completion_cell} | {cached_cell} | {cost_cell} | "
            + " | ".join(egress_cells)
            + " | "
            + ("; ".join(judge) if judge else "ok")
            + " |"
        )

    if egress_rows is not None:
        lines += ["", "job stats ↔ egress（全部 harness 合计）"]
        diff_lines = 0
        for label, ours, theirs in (
            ("prompt", job_sums.prompt, total_egress.prompt),
            ("completion", job_sums.completion, total_egress.completion),
            ("cached", job_sums.cached, total_egress.cached),
        ):
            note = flag_diff(label, ours, theirs)
            if note:
                flags.append(note)
                diff_lines += 1
        if not diff_lines:
            lines.append("ok，三项一致")

    if flags:
        lines += ["", "本 job 的红标："] + [f"- {item}" for item in flags]

    if detail:
        lines += [
            "",
            "| trial | task | agent | prompt | completion | cached | cost | reward | error |",
            "|---|---|---|---|---|---|---|---|---|",
        ]
        for record in records:
            context = record["context"] if isinstance(record["context"], dict) else {}
            lines.append(
                f"| {record['trial']} | {record['task']} | {record['agent']} | "
                f"{_int(context.get('n_input_tokens'))} | "
                f"{_int(context.get('n_output_tokens'))} | "
                f"{_int(context.get('n_cache_tokens'))} | "
                f"{_float(context.get('cost_usd'))} | "
                f"{_reward_text(record['reward'])} | {record['error'] or ''} |"
            )

    lines.append("")
    return lines


def render(
    jobs: list[tuple[str, dict[str, Any], dict[str, Any]]],
    egress_rows: list[dict[str, Any]] | None,
    *,
    detail: bool = False,
) -> str:
    lines = ["# Harness eval report", "", f"jobs: {len(jobs)}"]
    if egress_rows is None:
        lines.append("egress: 未提供（只用 Harbor 自己的数字）")
    else:
        states = {"ok": 0, "missing": 0, "n/a": 0}
        for row in egress_rows:
            state = row.get("usage")
            states[state if state in states else "n/a"] += 1
        lines.append(
            f"egress: {len(egress_rows)} 行（ok {states['ok']} / "
            f"missing {states['missing']} / n/a {states['n/a']}）"
        )
    lines += [
        "",
        "口径：token 只取 job 的 stats.* 与 trial 的 agent_result.*；prompt 含 cache。缺的写 缺失，不估。",
        "",
    ]
    for name, job, config in jobs:
        lines += render_job(name, job, config, egress_rows, detail=detail)
    return "\n".join(lines)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=(__doc__ or "").split("\n\n")[0])
    parser.add_argument(
        "--job-dir", action="append", required=True, help="job directory with result.json"
    )
    parser.add_argument("--egress", help="egress JSONL from the proxy")
    parser.add_argument("--out", help="write the report here instead of stdout")
    parser.add_argument("--detail", action="store_true", help="add one row per trial")
    args = parser.parse_args(argv)
    if hasattr(sys.stdout, "reconfigure"):
        # The report is UTF-8; a Windows console defaults to GBK and would
        # raise on the first arrow instead of printing the numbers.
        sys.stdout.reconfigure(encoding="utf-8")

    try:
        jobs = [
            (Path(path).name, *load_job(Path(path))) for path in args.job_dir
        ]
        egress_rows = load_egress(Path(args.egress)) if args.egress else None
    except (OSError, ValueError) as error:
        print(f"report: {error}", file=sys.stderr)
        return 2

    text = render(jobs, egress_rows, detail=args.detail)
    if args.out:
        Path(args.out).write_text(text + "\n", encoding="utf-8")
    else:
        sys.stdout.write(text + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())