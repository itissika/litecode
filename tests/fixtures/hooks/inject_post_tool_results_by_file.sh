#!/bin/sh
read -r payload || true
case "$payload" in
  *a.txt*)
    echo '{"injectMessages":[{"role":"user","content":"post-tool-inject-a","placement":"PostToolResults"}]}'
    ;;
  *b.txt*)
    echo '{"injectMessages":[{"role":"user","content":"post-tool-inject-b","placement":"PostToolResults"}]}'
    ;;
  *)
    echo '{}'
    ;;
esac
