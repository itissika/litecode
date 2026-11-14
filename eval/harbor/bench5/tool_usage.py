"""Structured tool-usage summaries for the eval collector.

This module reads only artifacts that a finished trial already contains:

* ``agent/litecode/diagnostics/<session>/tool-audit.jsonl``
* ``agent/trajectory.json`` observations

It never touches a running agent and never changes a model-visible result.
"""

from __future__ import annotations

import json
import re
from collections import Counter
from pathlib import Path
from typing import Any

REMINDER_ANCHOR = re.compile(r"^↳ #\d+ (tools\.[A-Za-z0-9_.]+):")
REMINDER_QUOTED = re.compile(r"^↳ '(tools\.[A-Za-z0-9_.]+)'")


def _strings(value: Any) -> list[str]:
    if not isinstance(value, list):
        return []
    return sorted(str(item) for item in value if isinstance(item, str))


def _tool_name(event: dict[str, Any]) -> str:
    value = event.get("tool")
    return value if isinstance(value, str) and value else "?"


def _caller(event: dict[str, Any]) -> str:
    value = event.get("caller")
    return value if isinstance(value, str) and value else "unknown"


def _array_value(event: dict[str, Any], key: str) -> list[str]:
    return _strings(event.get(key))

def summarize_tool_audit(trial_dir: Path) -> dict[str, Any]:
    """Merge start/exec_done audit rows into one record per call."""
    root = trial_dir / "agent/litecode/diagnostics"
    paths = sorted(root.glob("*/tool-audit.jsonl")) if root.is_dir() else []
    starts: dict[tuple[str, str], dict[str, Any]] = {}
    ends: dict[tuple[str, str], dict[str, Any]] = {}
    bad_lines = 0
    event_count = 0
    for path in paths:
        try:
            lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
        except OSError:
            continue
        for line in lines:
            if not line.strip():
                continue
            try:
                event = json.loads(line)
            except ValueError:
                bad_lines += 1
                continue
            if not isinstance(event, dict):
                bad_lines += 1
                continue
            call_id = event.get("call_id")
            session_id = event.get("session_id")
            if not isinstance(call_id, str) or not call_id:
                bad_lines += 1
                continue
            if not isinstance(session_id, str):
                session_id = "?"
            event_count += 1
            key = (session_id, call_id)
            phase = event.get("phase")
            if phase == "start":
                starts.setdefault(key, event)
            elif phase == "exec_done":
                ends[key] = event
    calls: list[dict[str, Any]] = []
    for key in sorted(set(starts) | set(ends)):
        start = starts.get(key, {})
        end = ends.get(key, {})
        call = {
            "session_id": key[0],
            "call_id": key[1],
            "tool": _tool_name(end or start),
            "caller": _caller(start or end),
            "parent_call_id": start.get("parent_call_id") or end.get("parent_call_id"),
            "tool_path": start.get("tool_path") or end.get("tool_path"),
            "ok": end.get("ok"),
            "error_kind": end.get("error_kind"),
            "error_code": end.get("error_code"),
            "input_keys": _array_value(start, "input_keys"),
            "output_fields": _array_value(end, "output_fields"),
            "input_bytes": start.get("input_bytes"),
            "content_bytes": end.get("content_bytes"),
            "data_bytes": end.get("data_bytes"),
            "duration_ms": end.get("duration_ms"),
        }
        calls.append(call)
    return {
        "present": bool(paths),
        "files": len(paths),
        "events": event_count,
        "bad_lines": bad_lines,
        "n_calls": len(calls),
        "calls": calls,
    }

def summarize_calls(calls: list[dict[str, Any]]) -> dict[str, Any]:
    by_tool: dict[str, Counter[str]] = {}
    by_caller: Counter[str] = Counter()
    input_keys: Counter[str] = Counter()
    output_fields: Counter[str] = Counter()
    errors: Counter[str] = Counter()
    for call in calls:
        tool = str(call.get("tool") or "?")
        bucket = by_tool.setdefault(
            tool,
            Counter(calls=0, ok=0, failed=0, duration_ms=0, content_bytes=0, data_bytes=0),
        )
        bucket["calls"] += 1
        if call.get("ok") is True:
            bucket["ok"] += 1
        elif call.get("ok") is False:
            bucket["failed"] += 1
        for key in ("duration_ms", "content_bytes", "data_bytes"):
            value = call.get(key)
            if isinstance(value, int) and not isinstance(value, bool):
                bucket[key] += value
        by_caller[str(call.get("caller") or "unknown")] += 1
        for key in call.get("input_keys") or []:
            input_keys[f"{tool}.{key}"] += 1
        for key in call.get("output_fields") or []:
            output_fields[f"{tool}.{key}"] += 1
        if call.get("error_kind") or call.get("error_code"):
            code = str(call.get("error_code") or call.get("error_kind") or "error")
            errors[f"{tool}.{code}"] += 1
    tools = {
        tool: {
            "calls": bucket["calls"],
            "ok": bucket["ok"],
            "failed": bucket["failed"],
            "duration_ms": bucket["duration_ms"],
            "content_bytes": bucket["content_bytes"],
            "data_bytes": bucket["data_bytes"],
        }
        for tool, bucket in sorted(by_tool.items())
    }
    return {
        "calls_by_tool": {key: value["calls"] for key, value in tools.items()},
        "by_tool": tools,
        "by_caller": dict(by_caller.most_common()),
        "input_keys": dict(input_keys.most_common()),
        "output_fields": dict(output_fields.most_common()),
        "errors": dict(errors.most_common()),
    }

def classify_reminder(line: str) -> str:
    text = line.strip()
    if "result has no field" in text:
        return "contract:unknown_field"
    if "lands outside the workspace" in text:
        return "path_risk:outside"
    if "sensitive system location" in text:
        return "path_risk:sensitive"
    if ("session" in text and "still running" in text) or " is live;" in text:
        return "process:live_session"
    if "showing lines" in text or "continue with start_line" in text:
        return "read:truncated_page"
    if "output was clipped" in text:
        return "exec:output_clipped"
    if "failed(" in text and "kept going" in text:
        return "failure:swallowed"
    if "invalid_args" in text:
        return "contract:invalid_args"
    if "used instead of" in text or "adjusted to" in text:
        return "param:clamped"
    if "language server reported" in text:
        return "lsp:diagnostics"
    if "results were cut" in text:
        return "truncation:results"
    if "left out because" in text:
        return "read:skipped_context"
    if "shortened to fit" in text:
        return "read:lines_capped"
    if "paths outside the workspace" in text:
        return "glob:outside_paths"
    if "first" in text and "matches" in text:
        return "grep:truncated"
    if "first" in text and "paths" in text:
        return "glob:truncated"
    if "limit is already" in text:
        return "grep:limit_cap"
    if "context lines were omitted" in text:
        return "grep:context_omitted"
    if "output was capped" in text:
        return "grep:output_capped"
    return "other"


def reminder_tool(line: str) -> str:
    for pattern in (REMINDER_ANCHOR, REMINDER_QUOTED):
        match = pattern.search(line)
        if match:
            return match.group(1)
    return "unknown"

def summarize_reminders(steps: list[Any]) -> dict[str, Any]:
    lines = 0
    outputs = 0
    chars = 0
    by_category: Counter[str] = Counter()
    by_tool: Counter[str] = Counter()
    texts: set[str] = set()
    for step in steps:
        if not isinstance(step, dict):
            continue
        observation = step.get("observation")
        if not isinstance(observation, dict):
            continue
        for result in observation.get("results") or []:
            if not isinstance(result, dict):
                continue
            content = result.get("content")
            if not isinstance(content, str):
                continue
            found = 0
            for line in content.splitlines():
                if not line.startswith("↳"):
                    continue
                found += 1
                lines += 1
                chars += len(line)
                by_category[classify_reminder(line)] += 1
                by_tool[reminder_tool(line)] += 1
                texts.add(line.strip())
            if found:
                outputs += 1
    return {
        "lines": lines,
        "outputs_with_reminders": outputs,
        "chars": chars,
        "unique_texts": len(texts),
        "by_category": dict(by_category.most_common()),
        "by_tool": dict(by_tool.most_common()),
    }


def summarize_tool_usage(trial_dir: Path, trajectory: dict[str, Any]) -> dict[str, Any]:
    audit = summarize_tool_audit(trial_dir)
    reminders = trajectory.get("reminders")
    if not isinstance(reminders, dict):
        reminders = summarize_reminders([])
    aggregate = summarize_calls(audit.get("calls", []))
    return {
        "present": bool(audit.get("present")) or bool(reminders.get("lines")),
        "audit": audit,
        **aggregate,
        "reminders": reminders,
    }
