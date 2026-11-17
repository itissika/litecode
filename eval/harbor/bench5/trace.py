#!/usr/bin/env python3
"""Normalize native harness traces for the bench5 panel.

The common ATIF ``agent/trajectory.json`` is a lossy view: it keeps tool calls
and observations, but several adapters drop the reasoning text.  Every harness
already writes a richer native log, and this module reads those logs instead of
inferring anything from the lossy view:

* LiteCode  ``agent/litecode/sessions.db`` + ``logs/litecode.log`` + tool audit
* Codex     ``agent/sessions/**/rollout-*.jsonl``
* Pi        ``agent/pi/sessions/*.jsonl``
* Opencode  ``agent/trajectory.json`` (steps carry reasoning_content)

The output is one normalized step list plus a summary.  The summary is meant to
be compared across harnesses; the step list is meant to be inspected when a
number looks suspicious.

A missing reasoning body is recorded as missing.  It is never filled with zero
or estimated from another harness, because a silent fill is exactly how the
"only LiteCode has reasoning" mistake happened.

Usage::

    python3 eval/harbor/bench5/trace.py summary <trial-dir> --harness litecode
    python3 eval/harbor/bench5/trace.py steps <trial-dir> --harness codex --step 23
    python3 eval/harbor/bench5/trace.py compare <harness=trial-dir> ...
    python3 eval/harbor/bench5/trace.py scan ~/eval/jobs/bench5 --task regex-chess
"""

from __future__ import annotations

import argparse
import json
import re
import sqlite3
import statistics
import sys
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any, Iterable

try:
    from tool_usage import summarize_tool_audit
except ImportError:  # when imported as eval.harbor.bench5.trace
    from .tool_usage import summarize_tool_audit

HARNESSES = ("litecode", "codex", "pi", "opencode")
SCHEMA_VERSION = 1

RUN_CODE_HEADER = re.compile(
    r"^run_code (?:ok|failed) · (?P<calls>\d+) calls · (?P<ms>\d+)ms"
)
REMINDER_LINE = re.compile(r"^↳ #(?P<index>\d+) (?P<tool>[^:]+): (?P<text>.*)$")
NESTED_CALL = re.compile(r"(?<![A-Za-z0-9_])tools\.([A-Za-z0-9_.]+)\(")
WALL_TIME = re.compile(r"Wall time: (?P<seconds>[0-9.]+) seconds")
EXIT_CODE = re.compile(r"(?:Process )?exited? with code (?P<code>\d+)")
TOKEN_COUNT = re.compile(r"Original token count: (?P<count>\d+)")
LIVE_SESSION = re.compile(r"session (?P<id>[A-Za-z0-9_.-]+) is live")
PROCESS_RUNNING = re.compile(r"Process running with session ID (?P<id>[A-Za-z0-9_.-]+)")
CLAMPED = re.compile(r"used instead of")
TIMEOUT = re.compile(r"timeout after|timed out")


def _read_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None


def _read_jsonl(path: Path) -> list[dict[str, Any]]:
    out: list[dict[str, Any]] = []
    try:
        lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError:
        return out
    for line in lines:
        line = line.strip()
        if not line:
            continue
        try:
            value = json.loads(line)
        except ValueError:
            continue
        if isinstance(value, dict):
            out.append(value)
    return out


def _int(value: Any) -> int | None:
    if isinstance(value, bool):
        return None
    if isinstance(value, int) and value >= 0:
        return value
    if isinstance(value, float) and value >= 0:
        return int(value)
    return None


def _float(value: Any) -> float | None:
    if isinstance(value, bool):
        return None
    if isinstance(value, (int, float)) and value >= 0:
        return float(value)
    return None


def _content_text(parts: Any) -> str:
    """Concatenate text/thinking parts from a Responses/Pi content list."""
    if not isinstance(parts, list):
        return ""
    chunks: list[str] = []
    for part in parts:
        if not isinstance(part, dict):
            continue
        for key in ("text", "thinking"):
            value = part.get(key)
            if isinstance(value, str) and value:
                chunks.append(value)
    return "\n".join(chunks)


def _json_text(value: Any) -> str:
    if isinstance(value, str):
        return value
    if value is None:
        return ""
    return json.dumps(value, ensure_ascii=False)


def _parse_json_args(value: Any) -> Any:
    if isinstance(value, dict):
        return value
    if isinstance(value, str):
        try:
            parsed = json.loads(value)
        except ValueError:
            return value
        return parsed
    return value


def _reward(trial_dir: Path) -> float | None:
    result = _read_json(trial_dir / "result.json")
    if not isinstance(result, dict):
        return None
    verifier = result.get("verifier_result")
    rewards = verifier.get("rewards") if isinstance(verifier, dict) else None
    if not isinstance(rewards, dict):
        return None
    value = rewards.get("reward")
    return _float(value)


def _execution_seconds(trial_dir: Path) -> float | None:
    result = _read_json(trial_dir / "result.json")
    if not isinstance(result, dict):
        return None
    execution = result.get("agent_execution")
    if not isinstance(execution, dict):
        return None
    from datetime import datetime

    def parse(value: Any) -> datetime | None:
        if not isinstance(value, str) or not value:
            return None
        try:
            return datetime.fromisoformat(value.replace("Z", "+00:00"))
        except ValueError:
            return None

    start, end = parse(execution.get("started_at")), parse(execution.get("finished_at"))
    if start is None or end is None:
        return None
    return max((end - start).total_seconds(), 0.0)


def detect_harness(trial_dir: Path) -> str | None:
    agent = trial_dir / "agent"
    if (agent / "litecode").is_dir():
        return "litecode"
    if (agent / "sessions").is_dir() and (agent / "codex.txt").exists():
        return "codex"
    if (agent / "pi").is_dir():
        return "pi"
    if (agent / "opencode").is_dir():
        return "opencode"
    if (agent / "trajectory.json").exists():
        # Opencode writes its reasoning into the ATIF file; the other harnesses
        # also write one, so only use this when nothing more specific exists.
        return "opencode"
    return None


def _new_step(
    *,
    step: int,
    timestamp: str | None = None,
    message: str = "",
    prompt_tokens: int | None = None,
    completion_tokens: int | None = None,
    cached_tokens: int | None = None,
    reasoning_tokens: int | None = None,
    reasoning_text: str = "",
    reasoning_complete: bool = True,
) -> dict[str, Any]:
    return {
        "step": step,
        "source": "agent",
        "timestamp": timestamp,
        "message": message,
        "prompt_tokens": prompt_tokens,
        "completion_tokens": completion_tokens,
        "cached_tokens": cached_tokens,
        "reasoning_tokens": reasoning_tokens,
        "reasoning_chars": len(reasoning_text),
        "reasoning_text": reasoning_text,
        "reasoning_complete": reasoning_complete,
        "tool_calls": [],
        "tool_results": [],
        "wall_time_s": None,
        "timeouts": 0,
        "live_sessions": 0,
        "clamped": 0,
        "errors": 0,
    }


def _add_result_flags(step: dict[str, Any], text: str) -> None:
    if TIMEOUT.search(text):
        step["timeouts"] += 1
    if LIVE_SESSION.search(text) or PROCESS_RUNNING.search(text):
        step["live_sessions"] += 1
    if CLAMPED.search(text):
        step["clamped"] += 1


# ---------------------------------------------------------------------------
# LiteCode
# ---------------------------------------------------------------------------


def _litecode_blob_text(litecode_dir: Path, body_ref: Any) -> tuple[str, bool]:
    if not isinstance(body_ref, str):
        return "", True
    match = re.search(r"\[blob:([A-Za-z0-9]+)\]", body_ref)
    if not match:
        return "", False
    blob_id = match.group(1)
    candidate = litecode_dir / "blobs" / blob_id[:2] / blob_id
    try:
        return candidate.read_text(encoding="utf-8", errors="replace"), True
    except OSError:
        return "", False


def _litecode_steps(trial_dir: Path) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    agent = trial_dir / "agent"
    log_path = agent / "litecode/logs/litecode.log"
    built: dict[int, dict[str, Any]] = {}
    done: dict[int, dict[str, Any]] = {}
    iterations: dict[int, dict[str, Any]] = {}
    audits: dict[int, list[dict[str, Any]]] = defaultdict(list)
    for event in _read_jsonl(log_path):
        fields = event.get("fields")
        if not isinstance(fields, dict):
            continue
        message = fields.get("message")
        step = _int(fields.get("step"))
        if message == "LLM request built" and step is not None:
            built[step] = fields
        elif message == "LLM request completed" and step is not None:
            done[step] = fields
        elif message == "agent loop iteration" and step is not None:
            iterations[step] = fields
        elif message == "tool_audit" and fields.get("phase") == "exec_done":
            call_step = _int(fields.get("step"))
            if call_step is not None:
                audits[call_step].append(fields)

    db_path = agent / "litecode/sessions.db"
    rows: list[dict[str, Any]] = []
    headers: dict[int, int] = {}
    missing_blobs = 0
    if db_path.is_file():
        try:
            connection = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
            connection.row_factory = sqlite3.Row
            rows = [
                dict(row)
                for row in connection.execute(
                    "select seq, item_type, token_estimate, body, body_ref "
                    "from transcript_items order by seq"
                )
            ]
            for row in rows:
                if row.get("item_type") != "request/header" or not row.get("body"):
                    continue
                try:
                    header = json.loads(row["body"])
                except (TypeError, ValueError):
                    continue
                step = _int(header.get("step"))
                if step is not None:
                    headers[step] = int(row["seq"])
            connection.close()
        except sqlite3.Error:
            rows = []

    steps: list[dict[str, Any]] = []
    header_seqs = sorted(headers.values())
    for step_number in sorted(done):
        header_seq = headers.get(step_number)
        if header_seq is None:
            continue
        next_seq = next((seq for seq in header_seqs if seq > header_seq), 10**18)
        items = [row for row in rows if header_seq < int(row["seq"]) < next_seq]
        usage = done[step_number]
        reasoning_parts: list[str] = []
        reasoning_tokens = 0
        complete = True
        calls: list[dict[str, Any]] = []
        results: list[dict[str, Any]] = []
        messages: list[str] = []
        for item in items:
            item_type = item.get("item_type")
            if item_type == "reasoning":
                reasoning_tokens += _int(item.get("token_estimate")) or 0
                text = ""
                if item.get("body"):
                    try:
                        body = json.loads(item["body"])
                    except (TypeError, ValueError):
                        body = {}
                    text = _content_text(body.get("content"))
                elif item.get("body_ref"):
                    text, found = _litecode_blob_text(agent / "litecode", item.get("body_ref"))
                    if not found:
                        missing_blobs += 1
                        complete = False
                if text:
                    reasoning_parts.append(text)
            elif item_type == "function_call" and item.get("body"):
                try:
                    body = json.loads(item["body"])
                except (TypeError, ValueError):
                    body = {}
                arguments = _parse_json_args(body.get("arguments"))
                nested = []
                code = arguments.get("code") if isinstance(arguments, dict) else None
                if isinstance(code, str):
                    for name in NESTED_CALL.findall(code):
                        nested.append({"name": name, "arguments": {}})
                calls.append(
                    {
                        "name": body.get("name") or "?",
                        "arguments": arguments,
                        "call_id": body.get("call_id"),
                        "nested": nested,
                    }
                )
            elif item_type == "function_call_output":
                raw = item.get("body")
                call_id = None
                try:
                    body = json.loads(raw) if raw else {}
                except (TypeError, ValueError):
                    body = {}
                if isinstance(body, dict):
                    call_id = body.get("call_id")
                    text = _json_text(body.get("output"))
                else:
                    text = _json_text(raw)
                results.append(
                    {
                        "call_id": call_id,
                        "name": None,
                        "content": text,
                        "chars": len(text),
                        "duration_ms": None,
                        "exit_code": None,
                        "error": None,
                    }
                )
            elif item_type == "message" and item.get("body"):
                try:
                    body = json.loads(item["body"])
                except (TypeError, ValueError):
                    body = {}
                text = _content_text(body.get("content"))
                if text:
                    messages.append(text)
        reasoning_text = "\n".join(reasoning_parts)
        step = _new_step(
            step=step_number,
            timestamp=usage.get("timestamp"),
            message="\n".join(messages),
            prompt_tokens=_int(usage.get("prompt_tokens")),
            completion_tokens=_int(usage.get("completion_tokens")),
            cached_tokens=_int(usage.get("cache_hit_tokens")),
            reasoning_tokens=reasoning_tokens or None,
            reasoning_text=reasoning_text,
            reasoning_complete=complete,
        )
        step["tool_calls"] = calls
        step["tool_results"] = results
        step["built_token_count"] = _int(built.get(step_number, {}).get("token_count"))
        step["tool_count"] = _int(iterations.get(step_number, {}).get("tool_count"))
        durations = [
            _float(event.get("duration_ms")) or 0.0 for event in audits.get(step_number, [])
        ]
        step["wall_time_s"] = round(sum(durations) / 1000.0, 3) if durations else None
        step["errors"] = sum(
            1 for event in audits.get(step_number, []) if event.get("ok") is False
        )
        for result in results:
            text = result["content"]
            _add_result_flags(step, text)
            header = RUN_CODE_HEADER.search(text)
            if header:
                result["run_code_ms"] = int(header.group("ms"))
            wall = WALL_TIME.search(text)
            if wall:
                result["wall_time_s"] = float(wall.group("seconds"))
            exit_code = EXIT_CODE.search(text)
            if exit_code:
                result["exit_code"] = int(exit_code.group("code"))
            live = LIVE_SESSION.search(text) or PROCESS_RUNNING.search(text)
            if live:
                result["session_id"] = live.group("id")
        if step["wall_time_s"] is None:
            run_code_ms = [
                result.get("run_code_ms")
                for result in results
                if result.get("run_code_ms") is not None
            ]
            if run_code_ms:
                step["wall_time_s"] = round(sum(run_code_ms) / 1000.0, 3)
            else:
                step["wall_time_s"] = next(
                    (result.get("wall_time_s") for result in results if result.get("wall_time_s")),
                    None,
                )
        steps.append(step)

    try:
        audit_calls = summarize_tool_audit(trial_dir).get("calls") or []
    except Exception:
        audit_calls = []
    for step in steps:
        top_ids = {
            call.get("call_id")
            for call in step.get("tool_calls") or []
            if call.get("call_id")
        }
        relevant = [
            call
            for call in audit_calls
            if call.get("call_id") in top_ids or call.get("parent_call_id") in top_ids
        ]
        top_durations = [
            call.get("duration_ms")
            for call in relevant
            if call.get("call_id") in top_ids
            and isinstance(call.get("duration_ms"), (int, float))
        ]
        nested_durations = [
            call.get("duration_ms")
            for call in relevant
            if call.get("call_id") not in top_ids
            and isinstance(call.get("duration_ms"), (int, float))
        ]
        if top_durations:
            step["wall_time_s"] = round(sum(top_durations) / 1000.0, 3)
        elif nested_durations:
            step["wall_time_s"] = round(sum(nested_durations) / 1000.0, 3)
        step["errors"] = sum(1 for call in relevant if call.get("ok") is False)

    meta = {
        "native_trace": str((log_path).relative_to(trial_dir)) if log_path.is_file() else "",
        "reasoning_source": "litecode.sessions.db",
        "reasoning_missing_blobs": missing_blobs,
        "requests": len(done),
        "completion_includes_reasoning": True,
    }
    return steps, meta


# ---------------------------------------------------------------------------
# Codex
# ---------------------------------------------------------------------------


def _codex_rollout(trial_dir: Path) -> Path | None:
    root = trial_dir / "agent/sessions"
    candidates = sorted(
        root.rglob("rollout-*.jsonl"), key=lambda path: path.stat().st_size, reverse=True
    )
    return candidates[0] if candidates else None


def _codex_steps(trial_dir: Path) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    rollout = _codex_rollout(trial_dir)
    steps: list[dict[str, Any]] = []
    if rollout is None:
        return _codex_trajectory_fallback(trial_dir)

    pending_reasoning: list[str] = []
    pending_complete = True
    calls_by_id: dict[str, dict[str, Any]] = {}
    current: dict[str, Any] | None = None
    requests = 0
    for event in _read_jsonl(rollout):
        kind = event.get("type")
        payload = event.get("payload")
        if not isinstance(payload, dict):
            continue
        if kind == "response_item":
            item_type = payload.get("type")
            if item_type == "reasoning":
                text = _content_text(payload.get("content"))
                if not text:
                    text = _content_text(payload.get("summary"))
                    if text:
                        pending_complete = False
                pending_reasoning.append(text)
            elif item_type == "function_call":
                step = _new_step(
                    step=len(steps) + 1,
                    message="",
                    reasoning_text="\n".join(pending_reasoning),
                    reasoning_complete=pending_complete,
                )
                call = {
                    "name": payload.get("name") or "?",
                    "arguments": _parse_json_args(payload.get("arguments")),
                    "call_id": payload.get("call_id"),
                    "nested": [],
                }
                step["tool_calls"] = [call]
                calls_by_id[str(call["call_id"])] = step
                steps.append(step)
                current = step
                pending_reasoning = []
                pending_complete = True
            elif item_type == "function_call_output":
                call_id = payload.get("call_id")
                step = calls_by_id.get(str(call_id))
                text = _json_text(payload.get("output"))
                if step is None:
                    step = current or _new_step(step=len(steps) + 1)
                    if current is None:
                        steps.append(step)
                result = {
                    "call_id": call_id,
                    "name": step["tool_calls"][0]["name"] if step["tool_calls"] else None,
                    "content": text,
                    "chars": len(text),
                    "duration_ms": None,
                    "exit_code": None,
                    "error": None,
                }
                wall = WALL_TIME.search(text)
                if wall:
                    result["wall_time_s"] = float(wall.group("seconds"))
                exit_code = EXIT_CODE.search(text)
                if exit_code:
                    result["exit_code"] = int(exit_code.group("code"))
                live = LIVE_SESSION.search(text) or PROCESS_RUNNING.search(text)
                if live:
                    result["session_id"] = live.group("id")
                step["tool_results"].append(result)
                _add_result_flags(step, text)
                if result.get("exit_code") not in (None, 0):
                    step["errors"] += 1
            elif item_type == "message":
                text = _content_text(payload.get("content"))
                if text and current is not None:
                    current["message"] = (current["message"] + "\n" + text).strip()
        elif kind == "event_msg" and payload.get("type") == "token_count":
            requests += 1
            info = payload.get("info")
            if not isinstance(info, dict) or not steps:
                continue
            usage = info.get("last_token_usage")
            if not isinstance(usage, dict):
                continue
            step = steps[-1]
            step["prompt_tokens"] = _int(usage.get("input_tokens"))
            step["completion_tokens"] = _int(usage.get("output_tokens"))
            step["cached_tokens"] = _int(usage.get("cached_input_tokens"))
            step["reasoning_tokens"] = _int(usage.get("reasoning_output_tokens"))

    for step in steps:
        if step["wall_time_s"] is None:
            step["wall_time_s"] = next(
                (result.get("wall_time_s") for result in step["tool_results"] if result.get("wall_time_s")),
                None,
            )
    meta = {
        "native_trace": str(rollout.relative_to(trial_dir)),
        "reasoning_source": "codex.rollout",
        "reasoning_missing_blobs": 0,
        "requests": requests or len(steps),
        "completion_includes_reasoning": True,
    }
    return steps, meta


def _codex_trajectory_fallback(trial_dir: Path) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    data = _read_json(trial_dir / "agent/trajectory.json")
    steps: list[dict[str, Any]] = []
    if not isinstance(data, dict):
        return steps, {"native_trace": "", "reasoning_source": "missing", "reasoning_missing_blobs": 0}
    for index, raw in enumerate(data.get("steps") or [], start=1):
        if not isinstance(raw, dict) or raw.get("source") != "agent":
            continue
        step = _new_step(step=index, timestamp=raw.get("timestamp"), message=raw.get("message") or "")
        for call in raw.get("tool_calls") or []:
            step["tool_calls"].append(
                {
                    "name": call.get("function_name") or "?",
                    "arguments": call.get("arguments") or {},
                    "call_id": call.get("tool_call_id"),
                    "nested": [],
                }
            )
        for result in (raw.get("observation") or {}).get("results") or []:
            text = _json_text(result.get("content"))
            step["tool_results"].append(
                {
                    "call_id": result.get("source_call_id"),
                    "name": None,
                    "content": text,
                    "chars": len(text),
                    "duration_ms": None,
                    "exit_code": None,
                    "error": None,
                }
            )
            _add_result_flags(step, text)
        steps.append(step)
    return steps, {
        "native_trace": "agent/trajectory.json",
        "reasoning_source": "missing",
        "reasoning_missing_blobs": 0,
        "requests": len(steps),
        "completion_includes_reasoning": True,
    }


# ---------------------------------------------------------------------------
# Pi
# ---------------------------------------------------------------------------


def _pi_session(trial_dir: Path) -> Path | None:
    root = trial_dir / "agent/pi/sessions"
    candidates = sorted(root.glob("*.jsonl"), key=lambda path: path.stat().st_size, reverse=True)
    return candidates[0] if candidates else None


def _pi_steps(trial_dir: Path) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    session = _pi_session(trial_dir)
    steps: list[dict[str, Any]] = []
    if session is None:
        return steps, {"native_trace": "", "reasoning_source": "missing", "reasoning_missing_blobs": 0}
    events = _read_jsonl(session)
    tool_results: dict[str, dict[str, Any]] = {}
    for event in events:
        if event.get("type") != "message":
            continue
        message = event.get("message")
        if not isinstance(message, dict) or message.get("role") != "toolResult":
            continue
        call_id = message.get("toolCallId")
        text = _content_text(message.get("content"))
        result = {
            "call_id": call_id,
            "name": message.get("toolName"),
            "content": text,
            "chars": len(text),
            "duration_ms": _int(message.get("durationMs")),
            "exit_code": None,
            "error": "tool error" if message.get("isError") else None,
        }
        if isinstance(call_id, str):
            tool_results[call_id] = result
    requests = 0
    for event in events:
        if event.get("type") != "message":
            continue
        message = event.get("message")
        if not isinstance(message, dict) or message.get("role") != "assistant":
            continue
        requests += 1
        content = message.get("content")
        if not isinstance(content, list):
            continue
        reasoning = "\n".join(
            part.get("thinking", "")
            for part in content
            if isinstance(part, dict) and part.get("type") == "thinking" and part.get("thinking")
        )
        visible = "\n".join(
            part.get("text", "")
            for part in content
            if isinstance(part, dict) and part.get("type") == "text" and part.get("text")
        )
        calls = []
        for part in content:
            if not isinstance(part, dict) or part.get("type") != "toolCall":
                continue
            calls.append(
                {
                    "name": part.get("name") or "?",
                    "arguments": part.get("arguments") or {},
                    "call_id": part.get("id"),
                    "nested": [],
                }
            )
        usage = message.get("usage") or {}
        cached = _int(usage.get("cacheRead"))
        prompt = _int(usage.get("input"))
        if prompt is not None and cached is not None:
            prompt += cached
        step = _new_step(
            step=len(steps) + 1,
            timestamp=message.get("timestamp"),
            message=visible,
            prompt_tokens=prompt,
            completion_tokens=_int(usage.get("output")),
            cached_tokens=cached,
            reasoning_tokens=_int(usage.get("reasoning")),
            reasoning_text=reasoning,
            reasoning_complete=bool(reasoning) or not calls,
        )
        step["tool_calls"] = calls
        for call in calls:
            result = tool_results.get(str(call.get("call_id")))
            if result:
                step["tool_results"].append(result)
                _add_result_flags(step, result["content"])
                if result.get("error"):
                    step["errors"] += 1
        durations = [
            result.get("duration_ms") or 0 for result in step["tool_results"]
        ]
        if durations:
            step["wall_time_s"] = round(sum(durations) / 1000.0, 3)
        steps.append(step)
    return steps, {
        "native_trace": str(session.relative_to(trial_dir)),
        "reasoning_source": "pi.session",
        "reasoning_missing_blobs": 0,
        "requests": requests,
        "completion_includes_reasoning": True,
    }


# ---------------------------------------------------------------------------
# Opencode
# ---------------------------------------------------------------------------


def _opencode_steps(trial_dir: Path) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    data = _read_json(trial_dir / "agent/trajectory.json")
    steps: list[dict[str, Any]] = []
    if not isinstance(data, dict):
        return steps, {"native_trace": "", "reasoning_source": "missing", "reasoning_missing_blobs": 0}
    requests = 0
    for raw in data.get("steps") or []:
        if not isinstance(raw, dict) or raw.get("source") != "agent":
            continue
        metrics = raw.get("metrics") or {}
        if _int(metrics.get("prompt_tokens")) is not None:
            requests += 1
        extra = metrics.get("extra") or {}
        calls = []
        for call in raw.get("tool_calls") or []:
            calls.append(
                {
                    "name": call.get("function_name") or "?",
                    "arguments": call.get("arguments") or {},
                    "call_id": call.get("tool_call_id"),
                    "nested": [],
                }
            )
        step = _new_step(
            step=len(steps) + 1,
            timestamp=raw.get("timestamp"),
            message=raw.get("message") or "",
            prompt_tokens=_int(metrics.get("prompt_tokens")),
            completion_tokens=_int(metrics.get("completion_tokens")),
            cached_tokens=_int(metrics.get("cached_tokens")),
            reasoning_tokens=_int(extra.get("reasoning_tokens")),
            reasoning_text=raw.get("reasoning_content") or "",
            reasoning_complete=bool(raw.get("reasoning_content")) or not calls,
        )
        step["tool_calls"] = calls
        for result in (raw.get("observation") or {}).get("results") or []:
            text = _json_text(result.get("content"))
            step["tool_results"].append(
                {
                    "call_id": result.get("source_call_id"),
                    "name": None,
                    "content": text,
                    "chars": len(text),
                    "duration_ms": None,
                    "exit_code": None,
                    "error": None,
                }
            )
            _add_result_flags(step, text)
        step["wall_time_s"] = _float(raw.get("duration_ms") or metrics.get("duration_ms"))
        steps.append(step)
    return steps, {
        "native_trace": "agent/trajectory.json",
        "reasoning_source": "opencode.trajectory",
        "reasoning_missing_blobs": 0,
        "requests": requests or len(steps),
        "completion_includes_reasoning": False,
    }


# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------


def _percentile(values: list[float], fraction: float) -> float | None:
    if not values:
        return None
    values = sorted(values)
    index = min(len(values) - 1, int(len(values) * fraction))
    return values[index]


def summarize_steps(
    harness: str,
    trial_dir: Path,
    steps: list[dict[str, Any]],
    meta: dict[str, Any],
) -> dict[str, Any]:
    reasoning_steps = [step for step in steps if step.get("reasoning_text")]
    reasoning_tokens = [
        value
        for value in (step.get("reasoning_tokens") for step in steps)
        if value is not None
    ]
    reasoning_chars = [step.get("reasoning_chars") or 0 for step in reasoning_steps]
    calls = Counter()
    nested = Counter()
    polls = 0
    timeouts = 0
    live_sessions = 0
    clamped = 0
    errors = 0
    for step in steps:
        for call in step.get("tool_calls") or []:
            name = call.get("name") or "?"
            calls[name] += 1
            for inner in call.get("nested") or []:
                inner_name = inner.get("name") or "?"
                nested[inner_name] += 1
                if inner_name in ("write_stdin", "process.write_stdin"):
                    polls += 1
            if name == "write_stdin":
                polls += 1
        timeouts += step.get("timeouts") or 0
        live_sessions += step.get("live_sessions") or 0
        clamped += step.get("clamped") or 0
        errors += step.get("errors") or 0

    missing = [
        {
            "step": step.get("step"),
            "reasoning_tokens": step.get("reasoning_tokens"),
            "reasoning_chars": step.get("reasoning_chars"),
        }
        for step in steps
        if not step.get("reasoning_complete", True)
    ]
    top_reasoning = sorted(
        (
            {
                "step": step.get("step"),
                "tokens": step.get("reasoning_tokens"),
                "chars": step.get("reasoning_chars") or 0,
                "complete": step.get("reasoning_complete", True),
            }
            for step in reasoning_steps
        ),
        key=lambda item: (item.get("tokens") or item.get("chars") or 0),
        reverse=True,
    )[:5]
    top_results = sorted(
        (
            {
                "step": step.get("step"),
                "name": result.get("name"),
                "chars": result.get("chars") or 0,
                "exit_code": result.get("exit_code"),
                "session_id": result.get("session_id"),
            }
            for step in steps
            for result in step.get("tool_results") or []
        ),
        key=lambda item: item.get("chars") or 0,
        reverse=True,
    )[:5]

    prompt = sum(step.get("prompt_tokens") or 0 for step in steps)
    completion = sum(step.get("completion_tokens") or 0 for step in steps)
    cached = sum(step.get("cached_tokens") or 0 for step in steps)
    reasoning_total = sum(reasoning_tokens) or 0
    requests = meta.get("requests") or sum(
        1
        for step in steps
        if step.get("prompt_tokens") is not None
        or step.get("completion_tokens") is not None
    )
    missing_blobs = meta.get("reasoning_missing_blobs", 0)
    completion_includes_reasoning = bool(meta.get("completion_includes_reasoning", True))
    return {
        "schema_version": SCHEMA_VERSION,
        "harness": harness,
        "task": trial_dir.name.split("__", 1)[0],
        "trial": trial_dir.name,
        "passed": _reward(trial_dir),
        "steps": len(steps),
        "requests": requests,
        "prompt_tokens": prompt or None,
        "completion_tokens": completion or None,
        "output_tokens_total": (completion + (0 if completion_includes_reasoning else reasoning_total))
        or None,
        "completion_includes_reasoning": completion_includes_reasoning,
        "cached_tokens": cached or None,
        "reasoning_tokens": reasoning_total or None,
        "reasoning_tokens_max": max(reasoning_tokens) if reasoning_tokens else None,
        "reasoning_tokens_median": statistics.median(reasoning_tokens)
        if reasoning_tokens
        else None,
        "reasoning_chars": sum(reasoning_chars) or None,
        "reasoning_chars_max": max(reasoning_chars) if reasoning_chars else None,
        "reasoning_chars_median": statistics.median(reasoning_chars)
        if reasoning_chars
        else None,
        "reasoning_items": len(reasoning_steps),
        "steps_without_reasoning": len(steps) - len(reasoning_steps),
        "reasoning_source": meta.get("reasoning_source"),
        "reasoning_complete": meta.get("reasoning_source") not in (None, "missing")
        and missing_blobs == 0,
        "reasoning_missing": missing if missing_blobs else [],
        "reasoning_missing_blobs": missing_blobs,
        "reasoning_estimated": not reasoning_tokens and bool(reasoning_chars),
        "tool_calls": sum(calls.values()),
        "tool_calls_by_name": dict(calls),
        "nested_calls": sum(nested.values()),
        "nested_calls_by_name": dict(nested),
        "polls": polls,
        "timeouts": timeouts,
        "live_sessions": live_sessions,
        "clamped": clamped,
        "errors": errors,
        "wall_time_s": round(
            sum(step.get("wall_time_s") or 0 for step in steps), 3
        )
        or None,
        "agent_sec": _execution_seconds(trial_dir),
        "native_trace": meta.get("native_trace"),
        "top_reasoning": top_reasoning,
        "top_results": top_results,
    }


def load_trace(trial_dir: Path, harness: str | None = None) -> dict[str, Any]:
    trial_dir = trial_dir.resolve()
    harness = harness or detect_harness(trial_dir)
    if harness not in HARNESSES:
        raise ValueError(f"unknown harness {harness!r} for {trial_dir}")
    if harness == "litecode":
        steps, meta = _litecode_steps(trial_dir)
    elif harness == "codex":
        steps, meta = _codex_steps(trial_dir)
    elif harness == "pi":
        steps, meta = _pi_steps(trial_dir)
    else:
        steps, meta = _opencode_steps(trial_dir)
    summary = summarize_steps(harness, trial_dir, steps, meta)
    return {"summary": summary, "steps": steps}


def summarize_trace(trial_dir: Path, harness: str | None = None) -> dict[str, Any]:
    """Summary-only entry point for the report collector."""
    return load_trace(trial_dir, harness)["summary"]


# ---------------------------------------------------------------------------
# Formatting
# ---------------------------------------------------------------------------


def _fmt(value: Any, suffix: str = "") -> str:
    if value is None:
        return "—"
    if isinstance(value, bool):
        return "yes" if value else "no"
    if isinstance(value, float):
        return f"{value:.3f}{suffix}"
    return f"{value:,}{suffix}"


def format_summary(summary: dict[str, Any]) -> str:
    lines = [
        f"# {summary['harness']} / {summary['trial']}",
        "",
        "| field | value |",
        "|---|---|",
    ]
    fields = (
        ("passed", summary.get("passed")),
        ("steps", summary.get("steps")),
        ("requests", summary.get("requests")),
        ("prompt", summary.get("prompt_tokens")),
        ("completion", summary.get("completion_tokens")),
        ("cached", summary.get("cached_tokens")),
        ("reasoning tokens", summary.get("reasoning_tokens")),
        ("reasoning chars", summary.get("reasoning_chars")),
        ("reasoning items", summary.get("reasoning_items")),
        ("reasoning max tokens", summary.get("reasoning_tokens_max")),
        ("reasoning max chars", summary.get("reasoning_chars_max")),
        ("reasoning source", summary.get("reasoning_source")),
        ("reasoning complete", summary.get("reasoning_complete")),
        ("missing reasoning", len(summary.get("reasoning_missing") or [])),
        ("missing blobs", summary.get("reasoning_missing_blobs")),
        ("tool calls", summary.get("tool_calls")),
        ("nested calls", summary.get("nested_calls")),
        ("polls", summary.get("polls")),
        ("timeouts", summary.get("timeouts")),
        ("live sessions", summary.get("live_sessions")),
        ("clamped", summary.get("clamped")),
        ("errors", summary.get("errors")),
        ("wall time", summary.get("wall_time_s")),
        ("agent time", summary.get("agent_sec")),
        ("native trace", summary.get("native_trace")),
    )
    for name, value in fields:
        lines.append(f"| {name} | {_fmt(value)} |")
    if summary.get("top_reasoning"):
        lines += ["", "## Top reasoning", "", "| step | tokens | chars | complete |", "|---:|---:|---:|:---:|"]
        for item in summary["top_reasoning"]:
            lines.append(
                f"| {item['step']} | {_fmt(item.get('tokens'))} | {_fmt(item.get('chars'))} | {'yes' if item.get('complete') else 'NO'} |"
            )
    if summary.get("top_results"):
        lines += ["", "## Largest tool results", "", "| step | name | chars | exit | session |", "|---:|---|---:|---:|---|"]
        for item in summary["top_results"]:
            lines.append(
                f"| {item['step']} | {item.get('name') or '—'} | {_fmt(item.get('chars'))} | {_fmt(item.get('exit_code'))} | {item.get('session_id') or '—'} |"
            )
    return "\n".join(lines)


def format_compare(summaries: Iterable[dict[str, Any]]) -> str:
    summaries = list(summaries)
    lines = [
        "| harness | steps | requests | prompt | output | output+reason | reasoning | max reason | polls | timeouts | live | clamp | pass | trace |",
        "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|:---:|---|",
    ]
    for summary in summaries:
        trace = "complete" if summary.get("reasoning_complete") else "partial"
        if summary.get("reasoning_source") in (None, "missing"):
            trace = "missing"
        lines.append(
            "| {harness} | {steps} | {requests} | {prompt} | {completion} | {output_total} | {reasoning} | {maxr} | {polls} | {timeouts} | {live} | {clamp} | {passed} | {trace} |".format(
                harness=summary.get("harness"),
                steps=_fmt(summary.get("steps")),
                requests=_fmt(summary.get("requests")),
                prompt=_fmt(summary.get("prompt_tokens")),
                completion=_fmt(summary.get("completion_tokens")),
                output_total=_fmt(summary.get("output_tokens_total")),
                reasoning=_fmt(summary.get("reasoning_tokens")),
                maxr=_fmt(summary.get("reasoning_tokens_max")),
                polls=_fmt(summary.get("polls")),
                timeouts=_fmt(summary.get("timeouts")),
                live=_fmt(summary.get("live_sessions")),
                clamp=_fmt(summary.get("clamped")),
                passed="yes" if summary.get("passed") else "no",
                trace=trace,
            )
        )
    lines += [
        "",
        "`output` is the raw completion count; `output+reason` adds reasoning when the harness reports it separately (opencode).",
    ]
    return "\n".join(lines)


def format_steps(steps: list[dict[str, Any]], step: int | None = None, full: bool = False) -> str:
    selected = [item for item in steps if step is None or item.get("step") == step]
    lines = [
        "| step | prompt | output | reason tok | reason chars | calls | results | wall s | flags |",
        "|---:|---:|---:|---:|---:|---|---:|---:|---|",
    ]
    for item in selected:
        calls = []
        for call in item.get("tool_calls") or []:
            name = call.get("name") or "?"
            nested = call.get("nested") or []
            calls.append(name + ("(" + ",".join(inner.get("name", "?") for inner in nested) + ")" if nested else ""))
        flags = []
        if item.get("timeouts"):
            flags.append(f"timeout×{item['timeouts']}")
        if item.get("live_sessions"):
            flags.append(f"live×{item['live_sessions']}")
        if item.get("clamped"):
            flags.append(f"clamp×{item['clamped']}")
        if item.get("errors"):
            flags.append(f"err×{item['errors']}")
        if not item.get("reasoning_complete", True):
            flags.append("reasoning-missing")
        lines.append(
            f"| {item.get('step')} | {_fmt(item.get('prompt_tokens'))} | {_fmt(item.get('completion_tokens'))} | "
            f"{_fmt(item.get('reasoning_tokens'))} | {_fmt(item.get('reasoning_chars'))} | "
            f"{' '.join(calls) or '—'} | {len(item.get('tool_results') or [])} | {_fmt(item.get('wall_time_s'))} | "
            f"{', '.join(flags) or '—'} |"
        )
    if step is not None:
        for item in selected:
            if item.get("reasoning_text"):
                text = item["reasoning_text"] if full else item["reasoning_text"][:2000]
                lines += ["", f"## step {item['step']} reasoning", "", text]
            for result in item.get("tool_results") or []:
                text = result.get("content") or ""
                text = text if full else text[:1000]
                lines += ["", f"## step {item['step']} result {result.get('call_id') or ''}", "", text]
    return "\n".join(lines)


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def _load_many(values: list[str]) -> list[dict[str, Any]]:
    summaries = []
    for value in values:
        if "=" in value:
            harness, path = value.split("=", 1)
        else:
            harness, path = None, value
        trace = load_trace(Path(path), harness)
        summaries.append(trace["summary"])
    return summaries


def _scan(root: Path, task: str | None, tag: str | None) -> list[Path]:
    pattern = f"bench5-{task}-*" if task else "bench5-*"
    if tag:
        pattern = f"{pattern}{tag}*"
    trials = []
    for job in sorted(root.glob(pattern)):
        if not job.is_dir():
            continue
        for trial in sorted(job.iterdir()):
            if not trial.is_dir() or not (trial / "agent").is_dir():
                continue
            if detect_harness(trial):
                trials.append(trial)
    return trials


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    summary = sub.add_parser("summary")
    summary.add_argument("trial")
    summary.add_argument("--harness", choices=HARNESSES)
    summary.add_argument("--json", action="store_true")

    steps = sub.add_parser("steps")
    steps.add_argument("trial")
    steps.add_argument("--harness", choices=HARNESSES)
    steps.add_argument("--step", type=int)
    steps.add_argument("--full", action="store_true")

    compare = sub.add_parser("compare")
    compare.add_argument("trials", nargs="+")
    compare.add_argument("--json", action="store_true")

    scan = sub.add_parser("scan")
    scan.add_argument("root", nargs="?", default=str(Path.home() / "eval/jobs/bench5"))
    scan.add_argument("--task")
    scan.add_argument("--tag")
    scan.add_argument("--json", action="store_true")

    args = parser.parse_args(argv)
    if args.command == "summary":
        trace = load_trace(Path(args.trial), args.harness)
        print(json.dumps(trace["summary"], ensure_ascii=False, indent=2) if args.json else format_summary(trace["summary"]))
        return 0
    if args.command == "steps":
        trace = load_trace(Path(args.trial), args.harness)
        print(format_steps(trace["steps"], args.step, args.full))
        return 0
    if args.command == "compare":
        summaries = _load_many(args.trials)
        print(json.dumps(summaries, ensure_ascii=False, indent=2) if args.json else format_compare(summaries))
        return 0
    trials = _scan(Path(args.root), args.task, args.tag)
    summaries = [load_trace(trial)["summary"] for trial in trials]
    print(json.dumps(summaries, ensure_ascii=False, indent=2) if args.json else format_compare(summaries))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
