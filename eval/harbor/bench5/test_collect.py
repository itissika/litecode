"""Tests for the bench5 collector's per-trial parsing and pricing."""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import collect  # noqa: E402


class CostTest(unittest.TestCase):
    def test_cost_uses_three_documented_prices(self) -> None:
        # 1,000 prompt: 500 uncached at 0.15/M + 500 cached at 0.003/M,
        # 100 output at 0.60/M.
        expected = (500 * 0.15 + 500 * 0.003 + 100 * 0.60) / 1_000_000
        self.assertAlmostEqual(collect.cost_of(1000, 500, 100), expected)

    def test_missing_token_stays_missing(self) -> None:
        self.assertIsNone(collect.cost_of(None, 0, 0))


class PerTrialParsingTest(unittest.TestCase):
    def test_reads_trial_result_not_the_job_rollup(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            job = Path(tmp) / "bench5-prove-plus-comm-r1"
            trial = job / "prove-plus-comm__abc123"
            (trial / "agent").mkdir(parents=True)
            (job / "config.json").write_text(
                json.dumps(
                    {
                        "job_name": "bench5-prove-plus-comm-r1",
                        "agents": [
                            {
                                "name": "eval.harbor.litecode_agent:LiteCode",
                                "model_name": "deepseek/deepseek-flash",
                                "model_api_key_name": "LITECODE_API_KEY",
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )
            # This is the trap: the job rollup has no trial_results.
            (job / "result.json").write_text(
                json.dumps({"stats": {"n_input_tokens": 999}}), encoding="utf-8"
            )
            (trial / "result.json").write_text(
                json.dumps(
                    {
                        "task_name": "terminal-bench/prove-plus-comm",
                        "trial_name": "prove-plus-comm__abc123",
                        "agent_info": {"name": "litecode"},
                        "agent_result": {
                            "n_input_tokens": 1000,
                            "n_cache_tokens": 800,
                            "n_output_tokens": 200,
                            "cost_usd": 0.0001206,
                        },
                        "verifier_result": {"rewards": {"reward": 1.0}},
                        "exception_info": None,
                        "started_at": "2026-11-13T00:00:00Z",
                        "finished_at": "2026-11-13T00:00:10Z",
                        "agent_setup": {
                            "started_at": "2026-11-13T00:00:00Z",
                            "finished_at": "2026-11-13T00:00:01Z",
                        },
                        "agent_execution": {
                            "started_at": "2026-11-13T00:00:01Z",
                            "finished_at": "2026-11-13T00:00:09Z",
                        },
                        "verifier": {
                            "started_at": "2026-11-13T00:00:09Z",
                            "finished_at": "2026-11-13T00:00:10Z",
                        },
                    }
                ),
                encoding="utf-8",
            )
            trail = (
                "[litecode] completed in 1.0s - tokens prompt=1000 completion=200 "
                "cache_hit=800 cache_miss=200\n"
            )
            (trial / "agent/litecode.txt").write_text(trail, encoding="utf-8")
            (trial / "agent/trajectory.json").write_text(
                json.dumps(
                    {
                        "steps": [
                            {"source": "agent", "llm_call_count": 1}
                        ]
                    }
                ),
                encoding="utf-8",
            )

            attempts = collect.load_attempts(Path(tmp), {})
            self.assertEqual(len(attempts), 1)
            cell = attempts[0][3]
            self.assertEqual(cell.task, "prove-plus-comm")
            self.assertEqual(cell.prompt, 1000)
            self.assertEqual(cell.cached, 800)
            self.assertEqual(cell.completion, 200)
            self.assertTrue(cell.valid)
            self.assertTrue(cell.clean)
            self.assertTrue(cell.passed)
            self.assertTrue(cell.rankable)
            self.assertAlmostEqual(
                cell.cost,
                (200 * collect.PRICE_UNCACHED_INPUT)
                + (800 * collect.PRICE_CACHED_INPUT)
                + (200 * collect.PRICE_OUTPUT),
            )

    def test_verifier_pass_with_nonzero_exit_is_valid_but_not_clean(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            job = Path(tmp) / "job"
            trial = job / "prove-plus-comm__x"
            (trial / "agent").mkdir(parents=True)
            (job / "config.json").write_text(
                json.dumps(
                    {
                        "job_name": "job",
                        "agents": [{"name": "opencode"}],
                    }
                ),
                encoding="utf-8",
            )
            (job / "result.json").write_text(
                json.dumps({"stats": {}}), encoding="utf-8"
            )
            (trial / "result.json").write_text(
                json.dumps(
                    {
                        "task_name": "terminal-bench/prove-plus-comm",
                        "trial_name": "prove-plus-comm__x",
                        "agent_info": {"name": "opencode"},
                        "agent_result": {},
                        "verifier_result": {"rewards": {"reward": 1.0}},
                        "exception_info": {
                            "exception_type": "NonZeroAgentExitCodeError",
                            "exception_message": "EPIPE",
                        },
                    }
                ),
                encoding="utf-8",
            )
            attempts = collect.load_attempts(Path(tmp), {})
            self.assertEqual(len(attempts), 1)
            cell = attempts[0][3]
            self.assertTrue(cell.valid)
            self.assertFalse(cell.clean)
            self.assertTrue(cell.passed)


class EgressClassificationTest(unittest.TestCase):
    def test_aborted_and_http_error_rows_are_warnings_not_missing_usage(self) -> None:
        rows = collect.EgressRows(
            "run",
            [
                {
                    "usage": "ok",
                    "status": 200,
                    "prompt_tokens": 100,
                    "cached_tokens": 80,
                    "completion_tokens": 20,
                },
                {
                    "usage": "missing",
                    "status": 400,
                    "error": "upstream 400 carried no usage",
                },
                {
                    "usage": "missing",
                    "status": 200,
                    "error": "BrokenPipeError: [Errno 32] Broken pipe",
                },
            ],
        )
        self.assertTrue(rows.token_complete)
        self.assertEqual(rows.ok_rows, 1)
        self.assertEqual(rows.http_errors, 1)
        self.assertEqual(rows.aborted, 1)
        self.assertEqual(rows.successful_missing, 0)
        self.assertEqual(rows.missing_usage, 2)

    def test_successful_2xx_without_usage_blocks_completeness(self) -> None:
        rows = collect.EgressRows(
            "run",
            [
                {
                    "usage": "ok",
                    "status": 200,
                    "prompt_tokens": 10,
                    "cached_tokens": 0,
                    "completion_tokens": 2,
                },
                {"usage": "missing", "status": 200, "error": None},
            ],
        )
        self.assertFalse(rows.token_complete)
        self.assertEqual(rows.successful_missing, 1)
if __name__ == "__main__":
    unittest.main()