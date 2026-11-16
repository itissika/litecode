"""Campaign manifest tests: matrix ownership, run-id budget, validation."""

from __future__ import annotations

import sys
import tempfile
import unittest
from dataclasses import replace
from pathlib import Path

import yaml

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

from campaign import CampaignError, load_campaign  # noqa: E402


def base_document() -> dict:
    return {
        "schema_version": 1,
        "name": "unit",
        "run_prefix": "unit",
        "model": "deepseek/deepseek-flash",
        "jobs_root": "/tmp/unit-jobs",
        "dataset": "/tmp/unit-dataset",
        "source_dataset": "/tmp/unit-source",
        "egress_log": "/tmp/unit-egress.jsonl",
        "egress_root": "http://127.0.0.1:8399",
        "harness_runtime": str(Path(tempfile.gettempdir()) / "unit-runtime"),
        "price_usd_per_million": {
            "uncached_input": 0.15,
            "cached_input": 0.003,
            "output": 0.60,
        },
        "tasks": [
            {
                "name": "task-a",
                "image": "litecode-eval/task-a:bench5",
                "source_image": "source/task-a:1",
            },
            {
                "name": "task-b",
                "image": "litecode-eval/task-b:bench5",
            },
        ],
        "harnesses": [
            {"id": "litecode", "token_source": "litecode"},
            {"id": "codex", "token_source": "egress", "probe": "codex --version"},
        ],
    }


class CampaignTest(unittest.TestCase):
    def write(self, document: dict) -> Path:
        tmp = tempfile.NamedTemporaryFile(
            "w", suffix=".yaml", delete=False, encoding="utf-8"
        )
        with tmp:
            yaml.safe_dump(document, tmp, sort_keys=False)
        self.addCleanup(Path(tmp.name).unlink, True)
        return Path(tmp.name)

    def test_matrix_is_data_not_hardcoded(self) -> None:
        campaign = load_campaign(self.write(base_document()))
        self.assertEqual(campaign.task_names, ("task-a", "task-b"))
        self.assertEqual(campaign.harness_ids, ("litecode", "codex"))
        self.assertEqual(
            campaign.expected_cells,
            {"task-a/litecode", "task-a/codex", "task-b/litecode", "task-b/codex"},
        )
        self.assertEqual(campaign.price["output"], 0.60)
        self.assertTrue(campaign.strict["require_token_complete"])
        self.assertEqual(campaign.harness_runtime.name, "unit-runtime")

    def test_duplicate_task_is_rejected(self) -> None:
        document = base_document()
        document["tasks"].append(
            {"name": "task-a", "image": "litecode-eval/task-a:bench5"}
        )
        with self.assertRaises(CampaignError):
            load_campaign(self.write(document))

    def test_unknown_token_source_is_rejected(self) -> None:
        document = base_document()
        document["harnesses"][0]["token_source"] = "self-report"
        with self.assertRaises(CampaignError):
            load_campaign(self.write(document))

    def test_egress_harness_needs_an_image_probe(self) -> None:
        document = base_document()
        document["harnesses"][1].pop("probe")
        with self.assertRaises(CampaignError):
            load_campaign(self.write(document))
    def test_run_id_leaves_room_for_longest_harness_suffix(self) -> None:
        campaign = load_campaign(self.write(base_document()))
        long = replace(
            campaign,
            run_prefix="campaign-" + "x" * 40,
            tasks=(replace(campaign.tasks[0], name="task-" + "y" * 40),),
        )
        run_id = long.run_id(long.tasks[0].name, "r1")
        self.assertLessEqual(len(run_id) + 1 + len("litecode"), 64)


if __name__ == "__main__":
    unittest.main()