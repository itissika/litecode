#!/bin/sh
# Consume stdin so the parent can close the pipe cleanly.
read -r _payload || true
printf '%s\n' '{"injectMessages":[{"role":"user","content":"post-tool-inject","placement":"PostToolResults"}]}'
