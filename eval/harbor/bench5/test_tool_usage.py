"""Tests for the tool-usage artifact summarizer."""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import tool_usage  # noqa: E402


class ToolAuditTest(unittest.TestCase):
    def test_merges_start_and_exec_done_without_values(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            trial = Path(tmp)
            directory = trial / "agent/litecode/diagnostics/s1"
            directory.mkdir(parents=True)
            events = [
                {
                    "phase": "start",
                    "session_id": "s1",
                    "tool": "exec_command",
                    "call_id": "p1:code:1",
                    "caller": "code",
                    "parent_call_id": "p1",
                    "tool_path": "process.exec_command",
                    "input_keys": ["cmd", "yield_time_ms"],
                    "input_bytes": 42,
                },
                {
                    "phase": "exec_done",
                    "session_id": "s1",
                    "tool": "exec_command",
                    "call_id": "p1:code:1",
                    "ok": True,
                    "output_fields": ["output"],
                    "content_bytes": 7,
                    "data_bytes": 11,
                    "duration_ms": 9,
                },
            ]
            (directory / "tool-audit.jsonl").write_text(
                "\n".join(json.dumps(event) for event in events) + "\nnot-json\n",
                encoding="utf-8",
            )
            summary = tool_usage.summarize_tool_audit(trial)
            self.assertTrue(summary["present"])
            self.assertEqual(summary["n_calls"], 1)
            self.assertEqual(summary["bad_lines"], 1)
            call = summary["calls"][0]
            self.assertEqual(call["tool"], "exec_command")
            self.assertEqual(call["caller"], "code")
            self.assertEqual(call["parent_call_id"], "p1")
            self.assertEqual(call["input_keys"], ["cmd", "yield_time_ms"])
            self.assertEqual(call["output_fields"], ["output"])
            self.assertEqual(call["duration_ms"], 9)

    def test_missing_diagnostics_is_not_an_error(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            summary = tool_usage.summarize_tool_audit(Path(tmp))
            self.assertFalse(summary["present"])
            self.assertEqual(summary["n_calls"], 0)


class ReminderTest(unittest.TestCase):
    def test_classifies_known_reminder_shapes(self) -> None:
        steps = [
            {
                "observation": {
                    "results": [
                        {
                            "content": (
                                "run_code ok\n\n"
                                "↳ #1 tools.file.write: this write lands outside the workspace: /tmp/x\n"
                                "↳ 'tools.process.exec_command' result has no field 'stderr'. fields: output\n"
                                "↳ #2 tools.process.exec_command: session is still running; use tools.process.write_stdin with its session_id\n"
                                "↳ #1 tools.file.read: showing lines 1-20 of 143; continue with start_line 21\n"
                                "↳ #1 tools.process.write_stdin: failed(not_found); the program kept going\n"
                            )
                        }
                    ]
                }
            }
        ]
        summary = tool_usage.summarize_reminders(steps)
        self.assertEqual(summary["lines"], 5)
        self.assertEqual(summary["outputs_with_reminders"], 1)
        self.assertEqual(summary["by_category"]["path_risk:outside"], 1)
        self.assertEqual(summary["by_category"]["contract:unknown_field"], 1)
        self.assertEqual(summary["by_category"]["process:live_session"], 1)
        self.assertEqual(summary["by_category"]["read:truncated_page"], 1)
        self.assertEqual(summary["by_category"]["failure:swallowed"], 1)
        self.assertEqual(summary["by_tool"]["tools.file.write"], 1)
        self.assertEqual(summary["by_tool"]["tools.process.exec_command"], 2)


class CallAggregateTest(unittest.TestCase):
    def test_counts_tools_inputs_outputs_and_errors(self) -> None:
        summary = tool_usage.summarize_calls(
            [
                {
                    "tool": "exec_command",
                    "caller": "code",
                    "ok": True,
                    "input_keys": ["cmd"],
                    "output_fields": ["output"],
                    "duration_ms": 5,
                    "content_bytes": 7,
                    "data_bytes": 9,
                },
                {
                    "tool": "write",
                    "caller": "code",
                    "ok": False,
                    "error_code": "io",
                    "input_keys": ["content", "path"],
                    "output_fields": [],
                },
            ]
        )
        self.assertEqual(summary["calls_by_tool"], {"exec_command": 1, "write": 1})
        self.assertEqual(summary["input_keys"]["exec_command.cmd"], 1)
        self.assertEqual(summary["input_keys"]["write.path"], 1)
        self.assertEqual(summary["output_fields"]["exec_command.output"], 1)
        self.assertEqual(summary["errors"]["write.io"], 1)
        self.assertEqual(summary["by_caller"]["code"], 2)


class ToolUsageTest(unittest.TestCase):
    def test_combines_audit_and_reminders(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            trial = Path(tmp)
            (trial / "agent").mkdir()
            summary = tool_usage.summarize_tool_usage(
                trial,
                {
                    "reminders": {
                        "lines": 1,
                        "outputs_with_reminders": 1,
                        "by_category": {"path_risk:outside": 1},
                        "by_tool": {"tools.file.write": 1},
                    }
                },
            )
            self.assertTrue(summary["present"])
            self.assertEqual(summary["reminders"]["lines"], 1)
            self.assertEqual(summary["calls_by_tool"], {})


if __name__ == "__main__":
    unittest.main()

class ReminderClassifyTest(unittest.TestCase):
    def test_live_session_text_with_id_is_still_live_session(self) -> None:
        line = (
            "\u21b3 #1 tools.process.exec_command: "
            "session bg_123 is live; use tools.process.write_stdin"
        )
        self.assertEqual(tool_usage.classify_reminder(line), "process:live_session")
