"""Tests for the native trace normalizer."""

from __future__ import annotations

import json
import sqlite3
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import trace  # noqa: E402


def write_jsonl(path: Path, values: list[dict]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        "\n".join(json.dumps(value, ensure_ascii=False) for value in values) + "\n",
        encoding="utf-8",
    )


def write_result(trial: Path, reward: float = 1.0) -> None:
    (trial / "result.json").write_text(
        json.dumps({"verifier_result": {"rewards": {"reward": reward}}}),
        encoding="utf-8",
    )


class LiteCodeTraceTest(unittest.TestCase):
    def test_reads_reasoning_calls_outputs_and_audit(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            trial = Path(tmp)
            write_result(trial)
            agent = trial / "agent/litecode"
            (agent / "logs").mkdir(parents=True)
            log = [
                {"fields": {"message": "LLM request built", "step": 1, "token_count": 100}},
                {
                    "fields": {
                        "message": "LLM request completed",
                        "step": 1,
                        "prompt_tokens": 200,
                        "completion_tokens": 10,
                        "cache_hit_tokens": 150,
                        "cache_miss_tokens": 50,
                    }
                },
            ]
            write_jsonl(agent / "logs/litecode.log", log)
            db = sqlite3.connect(agent / "sessions.db")
            db.execute(
                "create table transcript_items (seq integer, item_type text, token_estimate integer, body text, body_ref text)"
            )
            rows = [
                (0, "request/header", 0, json.dumps({"step": 1}), None),
                (
                    1,
                    "reasoning",
                    2,
                    json.dumps(
                        {
                            "type": "reasoning",
                            "content": [{"type": "reasoning_text", "text": "think"}],
                        }
                    ),
                    None,
                ),
                (
                    2,
                    "function_call",
                    5,
                    json.dumps(
                        {
                            "name": "run_code",
                            "arguments": json.dumps(
                                {
                                    "code": (
                                        "itertools.permutations('KQkq');\n"
                                        "await tools.process.write_stdin({session_id: 's'});"
                                    )
                                }
                            ),
                            "call_id": "c1",
                        }
                    ),
                    None,
                ),
                (
                    3,
                    "function_call_output",
                    20,
                    json.dumps(
                        {
                            "type": "function_call_output",
                            "call_id": "c1",
                            "output": (
                                "run_code ok · 1 calls · 60004ms\n\n"
                                "↳ #1 tools.process.write_stdin: session s is live; use tools.process.write_stdin"
                            ),
                        }
                    ),
                    None,
                ),
                (4, "request/header", 0, json.dumps({"step": 2}), None),
            ]
            db.executemany("insert into transcript_items values (?, ?, ?, ?, ?)", rows)
            db.commit()
            db.close()
            audit = [
                {
                    "phase": "start",
                    "session_id": "s1",
                    "tool": "run_code",
                    "call_id": "c1",
                    "caller": "direct",
                },
                {
                    "phase": "exec_done",
                    "session_id": "s1",
                    "tool": "run_code",
                    "call_id": "c1",
                    "ok": True,
                    "duration_ms": 60004,
                },
                {
                    "phase": "start",
                    "session_id": "s1",
                    "tool": "write_stdin",
                    "call_id": "c1:code:1",
                    "caller": "code",
                    "parent_call_id": "c1",
                },
                {
                    "phase": "exec_done",
                    "session_id": "s1",
                    "tool": "write_stdin",
                    "call_id": "c1:code:1",
                    "ok": True,
                    "duration_ms": 60000,
                },
            ]
            write_jsonl(agent / "diagnostics/s1/tool-audit.jsonl", audit)

            loaded = trace.load_trace(trial, "litecode")
            summary = loaded["summary"]
            self.assertEqual(summary["task"], trial.name.split("__")[0])
            self.assertEqual(summary["steps"], 1)
            self.assertEqual(summary["requests"], 1)
            self.assertEqual(summary["prompt_tokens"], 200)
            self.assertEqual(summary["reasoning_tokens"], 2)
            self.assertEqual(summary["tool_calls_by_name"], {"run_code": 1})
            self.assertEqual(summary["nested_calls_by_name"], {"process.write_stdin": 1})
            self.assertEqual(summary["polls"], 1)
            self.assertEqual(summary["live_sessions"], 1)
            self.assertEqual(summary["wall_time_s"], 60.004)
            self.assertTrue(summary["reasoning_complete"])

    def test_missing_blob_is_reported_not_zero_filled(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            trial = Path(tmp)
            agent = trial / "agent/litecode"
            (agent / "logs").mkdir(parents=True)
            write_jsonl(
                agent / "logs/litecode.log",
                [
                    {
                        "fields": {
                            "message": "LLM request completed",
                            "step": 1,
                            "prompt_tokens": 10,
                            "completion_tokens": 1,
                            "cache_hit_tokens": 0,
                            "cache_miss_tokens": 10,
                        }
                    }
                ],
            )
            db = sqlite3.connect(agent / "sessions.db")
            db.execute(
                "create table transcript_items (seq integer, item_type text, token_estimate integer, body text, body_ref text)"
            )
            db.executemany(
                "insert into transcript_items values (?, ?, ?, ?, ?)",
                [
                    (0, "request/header", 0, json.dumps({"step": 1}), None),
                    (1, "reasoning", 44133, None, "[blob:deadbeef]"),
                    (2, "request/header", 0, json.dumps({"step": 2}), None),
                ],
            )
            db.commit()
            db.close()

            summary = trace.load_trace(trial, "litecode")["summary"]
            self.assertFalse(summary["reasoning_complete"])
            self.assertEqual(summary["reasoning_tokens"], 44133)
            self.assertEqual(summary["reasoning_missing_blobs"], 1)
            self.assertEqual(summary["reasoning_missing"][0]["step"], 1)


class CodexTraceTest(unittest.TestCase):
    def test_reads_rollout_reasoning_and_last_usage(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            trial = Path(tmp)
            write_result(trial)
            rollout = trial / "agent/sessions/2026/11/14/rollout-1.jsonl"
            write_jsonl(
                rollout,
                [
                    {
                        "type": "response_item",
                        "payload": {
                            "type": "reasoning",
                            "content": [{"type": "reasoning_text", "text": "think"}],
                        },
                    },
                    {
                        "type": "response_item",
                        "payload": {
                            "type": "function_call",
                            "name": "exec_command",
                            "arguments": json.dumps({"cmd": "sleep 1"}),
                            "call_id": "c1",
                        },
                    },
                    {
                        "type": "response_item",
                        "payload": {
                            "type": "function_call_output",
                            "call_id": "c1",
                            "output": "Chunk ID: x\nWall time: 30.0003 seconds\nProcess running with session ID 7\nOutput:",
                        },
                    },
                    {
                        "type": "event_msg",
                        "payload": {
                            "type": "token_count",
                            "info": {
                                "last_token_usage": {
                                    "input_tokens": 100,
                                    "cached_input_tokens": 80,
                                    "output_tokens": 10,
                                    "reasoning_output_tokens": 5,
                                }
                            },
                        },
                    },
                ],
            )
            summary = trace.load_trace(trial, "codex")["summary"]
            self.assertEqual(summary["steps"], 1)
            self.assertEqual(summary["requests"], 1)
            self.assertEqual(summary["reasoning_tokens"], 5)
            self.assertEqual(summary["reasoning_chars"], 5)
            self.assertEqual(summary["polls"], 0)
            self.assertEqual(summary["live_sessions"], 1)
            self.assertEqual(summary["wall_time_s"], 30.0)
            self.assertTrue(summary["reasoning_complete"])


class PiTraceTest(unittest.TestCase):
    def test_reads_thinking_tool_call_and_usage(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            trial = Path(tmp)
            write_result(trial)
            session = trial / "agent/pi/sessions/s.jsonl"
            write_jsonl(
                session,
                [
                    {
                        "type": "message",
                        "message": {
                            "role": "assistant",
                            "content": [
                                {"type": "thinking", "thinking": "think"},
                                {
                                    "type": "toolCall",
                                    "id": "c1",
                                    "name": "bash",
                                    "arguments": {"command": "ls"},
                                },
                            ],
                            "usage": {
                                "input": 10,
                                "cacheRead": 5,
                                "output": 2,
                                "reasoning": 3,
                            },
                        },
                    },
                    {
                        "type": "message",
                        "message": {
                            "role": "toolResult",
                            "toolCallId": "c1",
                            "toolName": "bash",
                            "content": [{"type": "text", "text": "ok"}],
                            "durationMs": 7,
                            "isError": False,
                        },
                    },
                ],
            )
            summary = trace.load_trace(trial, "pi")["summary"]
            self.assertEqual(summary["steps"], 1)
            self.assertEqual(summary["requests"], 1)
            self.assertEqual(summary["prompt_tokens"], 15)
            self.assertEqual(summary["cached_tokens"], 5)
            self.assertEqual(summary["reasoning_tokens"], 3)
            self.assertEqual(summary["tool_calls_by_name"], {"bash": 1})
            self.assertEqual(summary["wall_time_s"], 0.007)
            self.assertTrue(summary["reasoning_complete"])


class OpencodeTraceTest(unittest.TestCase):
    def test_adds_reasoning_to_output_total(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            trial = Path(tmp)
            write_result(trial)
            (trial / "agent").mkdir(parents=True)
            (trial / "agent/trajectory.json").write_text(
                json.dumps(
                    {
                        "steps": [
                            {"source": "user", "message": "task"},
                            {
                                "source": "agent",
                                "reasoning_content": "think",
                                "tool_calls": [
                                    {
                                        "tool_call_id": "c1",
                                        "function_name": "bash",
                                        "arguments": {"command": "ls"},
                                    }
                                ],
                                "observation": {
                                    "results": [
                                        {"source_call_id": "c1", "content": "ok"}
                                    ]
                                },
                                "metrics": {
                                    "prompt_tokens": 100,
                                    "completion_tokens": 12,
                                    "cached_tokens": 80,
                                    "extra": {"reasoning_tokens": 7},
                                },
                            },
                        ]
                    }
                ),
                encoding="utf-8",
            )
            summary = trace.load_trace(trial, "opencode")["summary"]
            self.assertEqual(summary["steps"], 1)
            self.assertEqual(summary["requests"], 1)
            self.assertEqual(summary["output_tokens_total"], 19)
            self.assertFalse(summary["completion_includes_reasoning"])
            self.assertTrue(summary["reasoning_complete"])


class FormatTest(unittest.TestCase):
    def test_compare_names_output_plus_reasoning(self) -> None:
        text = trace.format_compare(
            [
                {
                    "harness": "opencode",
                    "steps": 1,
                    "requests": 1,
                    "prompt_tokens": 10,
                    "completion_tokens": 2,
                    "output_tokens_total": 5,
                    "reasoning_tokens": 3,
                    "reasoning_complete": True,
                }
            ]
        )
        self.assertIn("output+reason", text)
        self.assertIn("| 5 |", text)


if __name__ == "__main__":
    unittest.main()
