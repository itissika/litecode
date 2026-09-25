# Live view of LLM wire capture (see serve_win.ps1 -Wire).
#
# Tails <Dir>\<newest run>\index.jsonl: one line per LLM request, layered as
# sent (items, reasoning replay filled/empty, ciphertext, ids on the wire) and
# received (status, first byte, duration, reasoning/content/tool deltas, terminal).
#
# Usage (repo root, PowerShell):
#   ./scripts/wire_watch.ps1                 # newest run under .litecode\wire
#   ./scripts/wire_watch.ps1 -Session 01M3…  # only one session
#   ./scripts/wire_watch.ps1 -Json           # full summary records
#   ./scripts/wire_watch.ps1 -All            # replay the run from the start

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

Write-Host "==> waiting for a capture run under $Dir ..."
$run = $null
while (-not $run) {
  $run = Get-ChildItem -Path $Dir -Directory | Sort-Object Name -Descending | Select-Object -First 1
  if (-not $run) { Start-Sleep -Seconds 1 }
}
$index = Join-Path $run.FullName "index.jsonl"
Write-Host "==> run $($run.Name) — request bodies and raw SSE: $($run.FullName)"
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
