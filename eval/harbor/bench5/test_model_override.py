"""The LiteCode model override changes one agent, never the baseline."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import run_matrix  # noqa: E402


class LiteCodeModelOverrideTest(unittest.TestCase):
    def test_only_the_litecode_agent_changes(self) -> None:
        document = {
            "agents": [
                {
                    "name": run_matrix.LITECODE_AGENT,
                    "model_name": "deepseek/deepseek-flash",
                },
                {"name": "codex", "model_name": "deepseek/deepseek-flash"},
            ]
        }
        run_matrix.override_litecode_model(
            document, "cline/cline-pass/deepseek-v4.1-flash"
        )
        self.assertEqual(
            document["agents"][0]["model_name"],
            "cline/cline-pass/deepseek-v4.1-flash",
        )
        self.assertEqual(
            document["agents"][1]["model_name"], "deepseek/deepseek-flash"
        )

    def test_missing_litecode_agent_is_an_error(self) -> None:
        with self.assertRaises(RuntimeError):
            run_matrix.override_litecode_model(
                {"agents": []}, "cline/cline-pass/deepseek-v4.1-flash"
            )


if __name__ == "__main__":
    unittest.main()
