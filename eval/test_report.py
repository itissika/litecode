"""The report: two sources, and every place they disagree."""

from __future__ import annotations

import json
import shutil
import tempfile
import unittest
from pathlib import Path

import report


def trial(name: str, agent: str, prompt: int | None, completion: int | None, cached: int | None, *, error: str | None = None, reward: float | None = 1.0):
    record = {
        "task_name": name,
        "trial_name": f"{name}__{agent}",
        "agent_info": {"name": agent, "version": "x"},
    }
    if prompt is not None:
        record["agent_result"] = {
            "n_input_tokens": prompt,
            "n_output_tokens": completion,
            "n_cache_tokens": cached,
            "cost_usd": 0.001,
        }
    if reward is not None:
        record["verifier_result"] = {"rewards": {"reward": reward}}
    if error:
        record["exception_info"] = {"exception_type": error}
    return record


def config_for(document):
    names = []
    for trial in document.get("trial_results") or []:
        name = (trial.get("agent_info") or {}).get("name")
        if name and name not in names:
            names.append(name)
    return {"agents": [{"name": name} for name in names]}


def job(trials, *, prompt=0, completion=0, cached=0, cost=0.0):
    return {
        "stats": {
            "n_completed_trials": len(trials),
            "n_errored_trials": 0,
            "n_input_tokens": prompt,
            "n_output_tokens": completion,
            "n_cache_tokens": cached,
            "cost_usd": cost,
        },
        "trial_results": trials,
    }


def egress_row(run_id, prompt, completion, cached, usage="ok"):
    return {
        "run_id": run_id,
        "usage": usage,
        "prompt_tokens": prompt,
        "completion_tokens": completion,
        "cached_tokens": cached,
        "path": "/v1/chat/completions",
    }


class ReportTest(unittest.TestCase):
    def setUp(self):
        self.work = Path(tempfile.mkdtemp(prefix="report-test-"))
        self.addCleanup(shutil.rmtree, self.work, True)

    def write_job(self, name, document):
        path = self.work / name
        path.mkdir()
        (path / "result.json").write_text(json.dumps(document), encoding="utf-8")
        return path

    def test_two_sources_agree(self):
        document = job(
            [trial("t1", "pi", 100, 10, 80), trial("t2", "pi", 50, 5, 40)],
            prompt=150, completion=15, cached=120,
        )
        text = report.render([("job-a", document, config_for(document))],
            [egress_row("job-a-pi", 100, 10, 80), egress_row("job-a-pi", 50, 5, 40)],
        )
        self.assertIn("| pi | 2 | 150 | 15 | 120 |", text)
        self.assertIn("ok，三项一致", text)
        self.assertNotIn("⚠", text)

    def test_a_token_difference_is_red_with_the_delta(self):
        document = job([trial("t1", "pi", 100, 10, 80)], prompt=100, completion=10, cached=80)
        text = report.render([("job-a", document, config_for(document))], [egress_row("job-a-pi", 90, 10, 70)])
        self.assertIn("⚠ prompt 差 +10", text)
        self.assertIn("⚠ cached 差 +10", text)
        self.assertIn("本 job 的红标", text)

    def test_a_run_without_egress_rows_is_red(self):
        document = job([trial("t1", "codex", 10, 1, 8)], prompt=10, completion=1, cached=8)
        text = report.render([("job-a", document, config_for(document))], [])
        self.assertIn("⚠ 无 egress 行（run_id job-a-codex）", text)

    def test_an_egress_row_without_usage_is_red(self):
        document = job([trial("t1", "pi", 10, 1, 8)], prompt=10, completion=1, cached=8)
        text = report.render([("job-a", document, config_for(document))],
            [egress_row("job-a-pi", 10, 1, 8), egress_row("job-a-pi", None, None, None, usage="missing")],
        )
        self.assertIn("⚠ egress 有 1 行没拿到 usage", text)

    def test_missing_agent_result_and_job_stats_are_named_not_zeroed(self):
        document = job([trial("t1", "pi", None, None, None, error="AgentSetupTimeoutError")])
        document["stats"].update(
            {"n_input_tokens": None, "n_output_tokens": None, "n_cache_tokens": None, "cost_usd": None}
        )
        text = report.render([("job-a", document, config_for(document))], None)
        self.assertIn("⚠ 3 个 token 字段缺失", text)
        self.assertIn("⚠ job stats 缺 prompt", text)
        self.assertIn("缺失", text)
        self.assertIn("egress: 未提供", text)

    def test_a_declared_harness_without_trials_is_red(self):
        document = job([trial("t1", "pi", 10, 1, 8)], prompt=10, completion=1, cached=8)
        config = {"agents": [{"name": "pi"}, {"name": "codex"}]}
        text = report.render([("job-a", document, config)], None)
        self.assertIn("job 配了这一家，但没有任何 trial 记录", text)
        self.assertIn("| codex | 0 | 缺失 | 缺失 | 缺失 | 缺失 |", text)

    def test_detail_adds_one_row_per_trial(self):
        document = job([trial("t1", "pi", 10, 1, 8, reward=0.0)])
        text = report.render([("job-a", document, config_for(document))], None, detail=True)
        self.assertIn("| t1__pi | t1 | pi | 10 | 1 | 8 |", text)
        self.assertIn("reward=0.0", text)

    def test_main_writes_to_out_and_reports_a_broken_directory(self):
        document = job([trial("t1", "pi", 10, 1, 8)], prompt=10, completion=1, cached=8)
        path = self.write_job("job-a", document)
        out = self.work / "report.md"
        code = report.main(["--job-dir", str(path), "--out", str(out)])
        self.assertEqual(code, 0)
        self.assertIn("# Harness eval report", out.read_text(encoding="utf-8"))

        code = report.main(["--job-dir", str(self.work / "nope")])
        self.assertEqual(code, 2)


if __name__ == "__main__":
    unittest.main()