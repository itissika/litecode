#!/usr/bin/env bash
# Custom Tool wrapper for webfetch (see docs/examples/custom-tools.md).
# Protocol: read JSON from stdin; print result on stdout; exit 0 on success.
set -euo pipefail

if ! command -v curl >/dev/null 2>&1; then
  echo "curl is required" >&2
  exit 1
fi

input="$(cat)"

eval "$(INPUT="$input" python3 - <<'PY'
import json
import os
import shlex
import sys

raw = os.environ.get("INPUT", "")
try:
    data = json.loads(raw)
except json.JSONDecodeError:
    print('echo "invalid json input" >&2; exit 1', file=sys.stderr)
    sys.exit(1)

url = data.get("url")
if not url:
    print('echo "missing url" >&2; exit 1', file=sys.stderr)
    sys.exit(1)

fmt = data.get("format", "markdown")
print(f"url={shlex.quote(url)}")
print(f"format={shlex.quote(fmt)}")
PY
)"

curl_args=(-sL --max-time 30)
if [[ "$url" == *127.0.0.1* || "$url" == *localhost* ]]; then
  curl_args+=(--noproxy '*')
fi

if ! response="$(curl "${curl_args[@]}" "$url")"; then
  echo "curl error fetching $url" >&2
  exit 1
fi

FORMAT="$format" BODY="$response" python3 - <<'PY'
import html
import os
import re
import sys

fmt = os.environ["FORMAT"]
body = os.environ["BODY"]

def collapse_whitespace(text: str) -> str:
    lines = [line.rstrip() for line in text.splitlines()]
    out = []
    blank = 0
    for line in lines:
        if not line:
            blank += 1
            if blank <= 2:
                out.append("")
        else:
            blank = 0
            out.append(line)
    return "\n".join(out).rstrip()

def strip_html_tags(html_text: str) -> str:
    text = re.sub(r"<[^>]*>", "", html_text)
    return collapse_whitespace(text)

def html_to_markdown(html_text: str) -> str:
    text = html_text
    for level in range(1, 7):
        tag = f"h{level}"
        pattern = rf"(?i)<{tag}[^>]*>(.*?)</{tag}>"
        hashes = "#" * level
        text = re.sub(
            pattern,
            lambda m, h=hashes: f"{h} {m.group(1).strip()}",
            text,
        )
    text = re.sub(r"(?i)<li[^>]*>", "- ", text)
    text = re.sub(r"(?i)<br\s*/?>", "\n", text)
    text = re.sub(r"(?i)</p>", "\n\n", text)
    text = re.sub(r"<[^>]*>", "", text)
    for src, dst in (
        ("&amp;", "&"),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", '"'),
        ("&#39;", "'"),
        ("&nbsp;", " "),
    ):
        text = text.replace(src, dst)
    return collapse_whitespace(text)

if fmt == "html":
    out = body
elif fmt == "text":
    out = strip_html_tags(body)
else:
    out = html_to_markdown(body)

limit = 50_000
if len(out) > limit:
    out = f"{out[:limit]}... [truncated {len(out) - limit} bytes]"

print(out, end="")
PY
