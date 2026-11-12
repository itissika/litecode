"""The four layers, resolved: shapes, the thinking level, and the refusals."""

from __future__ import annotations

import unittest
from pathlib import Path

import yaml

import resolve

HERE = Path(__file__).resolve().parent
EGRESS = "http://egress.test:8399"
SKIP = object()


def rendered(**overrides):
    values = {
        "job": "compare",
        "dataset": "datasets/ds3",
        "run_id": "r1",
        "egress_base_url": EGRESS,
    }
    values.update(overrides)
    return resolve.resolve(**values)


class LayersTest(unittest.TestCase):
    def test_one_agent_per_harness_on_the_one_model(self) -> None:
        job = rendered()
        self.assertEqual(
            [agent["name"] for agent in job["agents"]],
            ["pi", "grok-build", "opencode"],
        )
        for agent in job["agents"]:
            self.assertEqual(agent["model_name"], "deepseek/deepseek-flash")
            self.assertEqual(agent["model_api_key_name"], "DEEPSEEK_API_KEY")
            self.assertIn("/r/r1-", agent["base_url"])
            self.assertTrue(agent["base_url"].endswith("/v1"))
        self.assertEqual(job["datasets"], [{"path": "datasets/ds3"}])

    def test_thinking_is_high_everywhere(self) -> None:
        agents = {agent["name"]: agent for agent in rendered()["agents"]}
        self.assertEqual(agents["pi"]["kwargs"]["reasoning_effort"], "high")
        self.assertEqual(agents["grok-build"]["kwargs"]["reasoning_effort"], "high")
        overlay = agents["opencode"]["kwargs"]["opencode_config"]
        self.assertEqual(
            overlay["provider"]["deepseek"]["models"]["deepseek-flash"]["options"][
                "reasoningEffort"
            ],
            "high",
        )

    def test_no_context_or_max_output_override_survives_resolution(self) -> None:
        job = rendered()
        self.assertEqual(
            sorted(set(resolve._walk(job)) & set(resolve._FORBIDDEN_KEYS)), []
        )
        self.assertNotIn("context_window", yaml.safe_dump(job))

    def test_localhost_is_refused_as_an_egress_root(self) -> None:
        with self.assertRaises(resolve.ResolveError) as caught:
            rendered(egress_base_url="http://localhost:8399")
        self.assertIn("localhost", str(caught.exception))

    def test_missing_egress_root_or_dataset_fails_loudly(self) -> None:
        for overrides in (
            {"egress_base_url": ""},
            {"dataset": ""},
            {"run_id": ""},
        ):
            with self.subTest(overrides=overrides):
                with self.assertRaises(resolve.ResolveError):
                    rendered(**overrides)

    def test_a_missing_layer_names_its_file(self) -> None:
        with self.assertRaises(resolve.ResolveError) as caught:
            rendered(job="nope")
        self.assertIn("nope.yaml", str(caught.exception))

    def test_the_rendered_job_is_a_harbor_job_config(self) -> None:
        try:
            from harbor.models.job.config import JobConfig
        except ImportError:  # the layer test needs no Harbor install
            self.skipTest("harbor is not importable here")
        job = JobConfig.model_validate(rendered())
        self.assertEqual([agent.name for agent in job.agents], ["pi", "grok-build", "opencode"])

    def test_the_layer_files_carry_no_endpoint_or_key(self) -> None:
        for path in sorted(HERE.rglob("*.yaml")):
            text = path.read_text(encoding="utf-8")
            for needle in ("localhost", "127.0.0.1", "172.", "sk-"):
                self.assertNotIn(needle, text, f"{path} holds {needle!r}")


if __name__ == "__main__":
    unittest.main()
