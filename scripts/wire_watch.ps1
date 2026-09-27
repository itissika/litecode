# Live view of LLM wire capture (see serve_win.ps1 -Wire).
#
# Tails <Dir>\<session>\summary.jsonl: one JSON record per LLM request.
# The human line is sent (items, reasoning replay, reminder count) plus
# received (status, timing, tokens). Raw bodies live beside it as
# <n>.<codec>.request.json and <n>.<codec>.response.sse.jsonl.
#
# Usage (repo root, PowerShell):
#   ./scripts/wire_watch.ps1                 # session folder most recently updated
#   ./scripts/wire_watch.ps1 -Session 01M3…  # that session's summary.jsonl
#   ./scripts/wire_watch.ps1 -Json           # full summary records
#   ./scripts/wire_watch.ps1 -All            # replay the summary from the start

param(
  [string]$Dir = "",
  [string]$Session = "",
  [switch]$Json,
  [switch]$All
)

$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
if (-not $Dir) { $Dir = Join-Path $Root ".litecode\wire" }
if (-not (Test-Path $Dir)) { throw "no capture dir: $Dir (start serve with -Wire)" }

Write-Host "==> waiting for a session summary under $Dir ..."
$run = $null
while (-not $run) {
  $dirs = @(Get-ChildItem -Path $Dir -Directory -ErrorAction SilentlyContinue)
  if ($Session) {
    $run = $dirs | Where-Object { $_.Name -eq $Session } | Select-Object -First 1
  } else {
    $run = $dirs |
      Where-Object { Test-Path (Join-Path $_.FullName "summary.jsonl") } |
      Sort-Object { (Get-Item (Join-Path $_.FullName "summary.jsonl")).LastWriteTime } -Descending |
      Select-Object -First 1
  }
  if (-not $run) { Start-Sleep -Seconds 1 }
}
$index = Join-Path $run.FullName "summary.jsonl"
Write-Host "==> session $($run.Name) — summary, request JSON, response SSE: $($run.FullName)"
while (-not (Test-Path $index)) { Start-Sleep -Milliseconds 500 }

$tail = if ($All) { @{} } else { @{ Tail = 0 } }
Get-Content -Path $index -Wait -Encoding utf8 @tail | ForEach-Object {
  if (-not $_.Trim()) { return }
  $record = $_ | ConvertFrom-Json
  if ($Session -and $record.session_id -ne $Session) { return }
  if ($Json) { $_; return }
  $status = $record.received.status
  $color = if ($null -eq $status -or $status -ge 400 -or $record.received.terminal -eq "none") { "Red" }
    elseif ($record.sent.assistant.reasoning_empty -gt 0) { "Yellow" }
    else { "Green" }
  Write-Host $record.line -ForegroundColor $color
}
