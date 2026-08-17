# Windows browser/Vite transition loop (HMR). Not the Electron end-state shell.
#
# Starts host cargo `litecode serve` + Vite, generates a session-local auth token,
# and prints a handshake URL you can open in a normal browser.
#
# Usage (from repo root, PowerShell):
#   ./scripts/serve_win.ps1
#   ./scripts/serve_win.ps1 -Workspace C:\path\to\project
#   ./scripts/serve_win.ps1 --workspace C:\path\to\project   # also accepted
#   ./scripts/serve_win.ps1 -ApiOnly
#   ./scripts/serve_win.ps1 -WebOnly          # API must already be running
#   ./scripts/serve_win.ps1 -Release
#   ./scripts/serve_win.ps1 -NoAuth           # open serve without LITECODE_TOKEN
#
# One process = one workspace. Changing folder requires restarting this script
# (no in-process hot switch). End-state Electron: ./scripts/dev_win.ps1
# Unix equivalent: ./scripts/serve.sh

param(
  [string]$Bind = $(if ($env:LITECODE_BIND) { $env:LITECODE_BIND } else { "127.0.0.1:7483" }),
  [string]$Agent = $(if ($env:LITECODE_AGENT) { $env:LITECODE_AGENT } else { "default" }),
  [string]$Workspace = $(if ($env:LITECODE_WORKSPACE) { $env:LITECODE_WORKSPACE } else { "" }),
  [int]$WebPort = $(if ($env:LITECODE_WEB_PORT) { [int]$env:LITECODE_WEB_PORT } else { 5179 }),
  [switch]$ApiOnly,
  [switch]$WebOnly,
  [switch]$Release,
  [switch]$NoAuth,
  [switch]$SkipNpmInstall,
  # Captures Unix-style leftovers like: --workspace C:\path (not bound as -Workspace)
  [Parameter(ValueFromRemainingArguments = $true)]
  [string[]]$RemainingArgs = @()
)

$ErrorActionPreference = "Stop"

# PowerShell does not treat `--workspace` like clap: unbound tokens become
# positional $Bind/$Agent. Recover that common invocation shape.
if ($Bind -eq "--workspace" -or $Bind -eq "-workspace") {
  if (-not $Workspace) { $Workspace = $Agent }
  $Bind = if ($env:LITECODE_BIND) { $env:LITECODE_BIND } else { "127.0.0.1:7483" }
  $Agent = if ($env:LITECODE_AGENT) { $env:LITECODE_AGENT } else { "default" }
}
for ($i = 0; $i -lt $RemainingArgs.Count; $i++) {
  $tok = $RemainingArgs[$i]
  if (($tok -eq "--workspace" -or $tok -eq "-workspace") -and ($i + 1) -lt $RemainingArgs.Count) {
    $Workspace = $RemainingArgs[$i + 1]
    $i++
  }
}
$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
$WebDir = Join-Path $Root "web"
$WebIndex = Join-Path $WebDir "dist\index.html"

if ($ApiOnly -and $WebOnly) {
  throw "-ApiOnly and -WebOnly are mutually exclusive"
}

function Test-Command([string]$Name) {
  return [bool](Get-Command $Name -ErrorAction SilentlyContinue)
}

function New-DevToken {
  $bytes = New-Object byte[] 32
  [System.Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
  return [Convert]::ToBase64String($bytes).TrimEnd('=').Replace('+', '-').Replace('/', '_')
}

function Wait-Health([string]$Url, [int]$TimeoutSec = 600) {
  $deadline = (Get-Date).AddSeconds($TimeoutSec)
  Write-Host "==> waiting for API at $Url ..."
  $n = 0
  while ((Get-Date) -lt $deadline) {
    try {
      $r = Invoke-WebRequest -Uri $Url -UseBasicParsing -TimeoutSec 2
      if ($r.StatusCode -eq 200) {
        Write-Host "==> API is ready"
        return
      }
    } catch {
      # still compiling / starting
    }
    $n++
    if (($n % 10) -eq 0) { Write-Host -NoNewline "." }
    Start-Sleep -Seconds 1
  }
  throw "timeout waiting for $Url (${TimeoutSec}s)"
}

if (-not (Test-Command "cargo")) {
  throw "cargo not found on PATH — install Rust MSVC toolchain first"
}
if (-not $ApiOnly -and -not (Test-Command "npm")) {
  throw "npm not found on PATH — install Node.js first"
}

# Serve needs a built web/dist even when Vite is the primary UI (static fallback).
if (-not $WebOnly -and -not (Test-Path $WebIndex)) {
  Write-Host "==> web/dist missing — building once for serve static fallback"
  Push-Location $WebDir
  try {
    if (-not (Test-Path "node_modules")) {
      if (Test-Path "package-lock.json") { npm ci } else { npm install }
    }
    npm run build
  } finally {
    Pop-Location
  }
}

$token = $null
if (-not $NoAuth) {
  if ($env:LITECODE_TOKEN -and $env:LITECODE_TOKEN.Trim().Length -gt 0) {
    $token = $env:LITECODE_TOKEN.Trim()
    Write-Host "==> using existing LITECODE_TOKEN from environment"
  } else {
    $token = New-DevToken
    $env:LITECODE_TOKEN = $token
  }
  $env:VITE_AUTH_TOKEN = $token
} else {
  Remove-Item Env:\LITECODE_TOKEN -ErrorAction SilentlyContinue
  Remove-Item Env:\VITE_AUTH_TOKEN -ErrorAction SilentlyContinue
}

$procs = @()
try {
  if (-not $WebOnly) {
    $cargoArgs = @("run")
    if ($Release) { $cargoArgs += "--release" }
    $cargoArgs += @("--")
    if ($Workspace -and $Workspace.Trim().Length -gt 0) {
      $ws = (Resolve-Path -LiteralPath $Workspace.Trim()).Path
      $cargoArgs += @("--workspace", $ws)
      Write-Host "==> workspace=$ws (process cwd will chdir here)"
    } else {
      Write-Host "==> workspace=cwd ($Root) — pass -Workspace to pin a project root"
    }
    $cargoArgs += @("serve", "--bind", $Bind, "--agent", $Agent)
    if (-not $NoAuth) { $cargoArgs += "--require-auth" }

    Write-Host "==> starting API via cargo $($cargoArgs -join ' ') (LITECODE_CHANNEL=dev)"
    Write-Host "    bind=$Bind agent=$Agent"
    $savedChannel = $env:LITECODE_CHANNEL
    $env:LITECODE_CHANNEL = "dev"
    $api = Start-Process -FilePath "cargo" -ArgumentList $cargoArgs `
      -WorkingDirectory $Root -NoNewWindow -PassThru
    if ($null -eq $savedChannel) {
      Remove-Item Env:\LITECODE_CHANNEL -ErrorAction SilentlyContinue
    } else {
      $env:LITECODE_CHANNEL = $savedChannel
    }
    $procs += $api

    $health = "http://$Bind/health"
    Wait-Health $health
  }

  if (-not $ApiOnly) {
    if (-not (Test-Path $WebDir)) { throw "web directory missing: $WebDir" }
    if (-not $SkipNpmInstall -and -not (Test-Path (Join-Path $WebDir "node_modules"))) {
      Write-Host "==> installing web dependencies"
      Push-Location $WebDir
      try {
        if (Test-Path "package-lock.json") { npm ci } else { npm install }
      } finally { Pop-Location }
    }

    Write-Host "==> starting Vite (proxies /api /ws /health → $Bind)"
    # Start-Process bypasses PowerShell's .ps1/.cmd command resolution. On
    # nvm-windows, the extensionless `npm` shim is not a Win32 executable;
    # invoke the Windows command shim explicitly.
    $vite = Start-Process -FilePath "npm.cmd" -ArgumentList @("run", "dev", "--", "--port", "$WebPort", "--strictPort") `
      -WorkingDirectory $WebDir -NoNewWindow -PassThru
    $procs += $vite
  }

  $ui = "http://127.0.0.1:$WebPort/"
  if ($token) {
    $handshake = "${ui}?token=$([uri]::EscapeDataString($token))"
  } else {
    $handshake = $ui
  }

  Write-Host ""
  Write-Host "============================================================"
  Write-Host "  LITECODE_BROWSER_DEV (open in browser)"
  Write-Host "  $handshake"
  Write-Host "============================================================"
  Write-Host "  API health: http://$Bind/health"
  Write-Host "  API ws:     ws://$Bind/ws"
  if ($token) {
    Write-Host "  token:      $token"
    Write-Host "  (also injected as VITE_AUTH_TOKEN for this Vite process)"
  } else {
    Write-Host "  auth:       off (-NoAuth)"
  }
  Write-Host "  Electron end-state: ./scripts/dev_win.ps1"
  Write-Host "============================================================"
  Write-Host ""
  Write-Host "Press Ctrl+C to stop."

  # Wait until any child exits (or Ctrl+C).
  while ($true) {
    foreach ($p in $procs) {
      if ($p.HasExited) {
        Write-Host "==> process exited (pid=$($p.Id) code=$($p.ExitCode))"
        exit $(if ($null -ne $p.ExitCode) { $p.ExitCode } else { 1 })
      }
    }
    Start-Sleep -Milliseconds 500
  }
} finally {
  foreach ($p in $procs) {
    if (-not $p.HasExited) {
      Write-Host "==> stopping pid $($p.Id)"
      try { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue } catch {}
      # cargo may leave litecode.exe; best-effort tree kill
      try {
        & taskkill /PID $p.Id /T /F 2>$null | Out-Null
      } catch {}
    }
  }
}
