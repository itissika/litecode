# Windows native Electron desktop dev loop.
#
# Electron still owns the sidecar (serve --bind 127.0.0.1:0 --require-auth, token
# injected). The workbench document is the Vite dev server, so web/ edits
# hot-reload inside the desktop window. The sidecar port changes on each
# workspace launch; the host writes it to dist/dev-sidecar-upstream.txt and Vite
# proxies /api /ws /health there.
#
# -NoHmr loads the sidecar's built web/dist instead (packaged shape).
# Browser-only Vite loop: scripts/serve_win.ps1 / scripts/serve.sh.
#
# Usage (from repo root, PowerShell):
#   ./scripts/dev_win.ps1
#   ./scripts/dev_win.ps1 -NoHmr               # static web/dist, no Vite
#   ./scripts/dev_win.ps1 -WebPort 5180        # Vite port (default 5179)
#   ./scripts/dev_win.ps1 -RebuildWeb          # refresh web/dist (sidecar still boots it)
#   ./scripts/dev_win.ps1 -Profile release
#   ./scripts/dev_win.ps1 -Cuda                # sidecar with ORT CUDA EP (CPU fallback)
#   ./scripts/dev_win.ps1 -SkipAssemble        # reuse existing dist/product
#   ./scripts/dev_win.ps1 -BundleModel         # run embed model bundler if needed
#
#   ./scripts/dev_win.ps1 -SkipLinuxBundle    # pure local; Open Remote will not work
#
# Prerequisites: Rust (MSVC), Node.js, VS Build Tools. Git Bash only needed if
# -BundleModel and scripts/bundle_embed_model.sh must run.
# Open Remote also needs dist/linux/ from WSL: ./scripts/package_linux.sh

param(
  [ValidateSet("debug", "release")]
  [string]$Profile = "debug",
  [switch]$RebuildWeb,
  [switch]$SkipAssemble,
  [switch]$BundleModel,
  [switch]$SkipNpmInstall,
  [switch]$SkipLinuxBundle,
  [switch]$Cuda,
  [switch]$NoHmr,
  [int]$WebPort = $(if ($env:LITECODE_WEB_PORT) { [int]$env:LITECODE_WEB_PORT } else { 5179 })
)

$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
$Product = Join-Path $Root "dist\product"
$WebIndex = Join-Path $Root "web\dist\index.html"
$Desktop = Join-Path $Root "desktop"
$SidecarExe = Join-Path $Product "litecode.exe"

function Test-Command([string]$Name) {
  return [bool](Get-Command $Name -ErrorAction SilentlyContinue)
}

function Restore-EnvValue([string]$Name, $Value) {
  if ($null -eq $Value -or "$Value" -eq "") {
    Remove-Item "Env:\$Name" -ErrorAction SilentlyContinue
  } else {
    Set-Item -Path "Env:\$Name" -Value $Value
  }
}

function Stop-DevProcess($Proc) {
  if ($null -eq $Proc) { return }
  if ($Proc.HasExited) { return }
  Write-Host "==> stopping pid $($Proc.Id)"
  try { Stop-Process -Id $Proc.Id -Force -ErrorAction SilentlyContinue } catch {}
  try { & taskkill /PID $Proc.Id /T /F 2>$null | Out-Null } catch {}
}

function Wait-Vite([string]$Url, $Proc, [int]$TimeoutSec = 60) {
  $deadline = (Get-Date).AddSeconds($TimeoutSec)
  Write-Host "==> waiting for Vite at $Url"
  while ((Get-Date) -lt $deadline) {
    if ($Proc.HasExited) {
      throw "Vite exited before it was ready (code=$($Proc.ExitCode))"
    }
    try {
      $response = Invoke-WebRequest -Uri $Url -UseBasicParsing -TimeoutSec 2
      if ($response.StatusCode -eq 200) {
        Write-Host "==> Vite is ready"
        return
      }
    } catch {
      # still starting
    }
    Start-Sleep -Milliseconds 300
  }
  throw "timeout waiting for Vite at $Url (${TimeoutSec}s)"
}

# Vite in front of the sidecar. Electron publishes the ephemeral port; this
# process only serves web/ and proxies /api /ws /health at that file.
function Start-DevVite([string]$RepoRoot, [int]$Port, [bool]$SkipInstall) {
  $webDir = Join-Path $RepoRoot "web"
  if (-not (Test-Path $webDir)) { throw "web directory missing: $webDir" }
  if (-not $SkipInstall -and -not (Test-Path (Join-Path $webDir "node_modules"))) {
    Write-Host "==> installing web dependencies"
    Push-Location $webDir
    try {
      # Out-Host: a function's success stream is its return value. npm must not
      # ride along with the process object.
      if (Test-Path "package-lock.json") { npm ci | Out-Host } else { npm install | Out-Host }
    } finally { Pop-Location }
  }

  $upstream = Join-Path $RepoRoot "dist\dev-sidecar-upstream.txt"
  New-Item -ItemType Directory -Force -Path (Join-Path $RepoRoot "dist") | Out-Null
  Set-Content -LiteralPath $upstream -Value "" -Encoding ascii
  $env:LITECODE_DEV_UPSTREAM_FILE = $upstream
  $env:LITECODE_UI_DEV_URL = "http://127.0.0.1:$Port/"

  Write-Host "==> starting Vite hot reload at $($env:LITECODE_UI_DEV_URL)"
  # npm.cmd: nvm-windows' extensionless npm shim is not a Win32 executable.
  $proc = Start-Process -FilePath "npm.cmd" -ArgumentList @("run", "dev", "--", "--port", "$Port", "--strictPort", "--host", "127.0.0.1") `
    -WorkingDirectory $webDir -NoNewWindow -PassThru
  try {
    Wait-Vite $env:LITECODE_UI_DEV_URL $proc
  } catch {
    Stop-DevProcess $proc
    throw
  }
  return ,$proc
}

if (-not (Test-Command "cargo")) {
  throw "cargo not found on PATH — install Rust MSVC toolchain first"
}
if (-not (Test-Command "npm")) {
  throw "npm not found on PATH — install Node.js first"
}

if (-not $SkipAssemble) {
  $needWeb = $RebuildWeb -or -not (Test-Path $WebIndex)
  $assembleArgs = @{
    Profile = $Profile
    SkipModel = (-not $BundleModel)
  }
  if ($Cuda) {
    if (-not $env:ORT_CUDA_VERSION) { $env:ORT_CUDA_VERSION = "12" }
    $assembleArgs.Features = "ort-cuda"
    $assembleArgs.TargetDir = (Join-Path $Root "target\cuda-accel")
    $assembleArgs.KeepCudaDylibs = $true
    Write-Host "==> CUDA sidecar (ort-cuda, KeepCudaDylibs, ORT_CUDA_VERSION=$($env:ORT_CUDA_VERSION))"
  }
  if (-not $needWeb) {
    $assembleArgs.SkipWeb = $true
    if ($NoHmr) {
      Write-Host "==> reusing web/dist (pass -RebuildWeb to rebuild UI)"
    } else {
      Write-Host "==> reusing web/dist (sidecar boot only; the window hot-reloads from Vite)"
    }
  } else {
    Write-Host "==> building web/dist"
  }

  & (Join-Path $Root "scripts\assemble_product.ps1") @assembleArgs
} else {
  Write-Host "==> skipping assemble (-SkipAssemble)"
}

if (-not (Test-Path $SidecarExe)) {
  throw "sidecar missing at $SidecarExe — run without -SkipAssemble, or run scripts/assemble_product.ps1 first"
}
if (-not (Test-Path (Join-Path $Product "web\dist\index.html"))) {
  throw "product UI missing under $Product\web\dist — rebuild with assemble (omit -SkipWeb / use -RebuildWeb)"
}

if (-not $SkipLinuxBundle) {
  $null = & (Join-Path $Root "scripts\ensure_linux_bundle.ps1") -Root $Root -Require -WarnOnly
} else {
  Write-Host "==> skipping Linux bundle check (-SkipLinuxBundle); Open Remote may fail"
}

$savedDevUrl = $env:LITECODE_DEV_URL
$savedUiDevUrl = $env:LITECODE_UI_DEV_URL
$savedUpstreamFile = $env:LITECODE_DEV_UPSTREAM_FILE
$savedSidecarDir = $env:LITECODE_SIDECAR_DIR
$vite = $null
Push-Location $Desktop
try {
  if (-not $SkipNpmInstall) {
    if (-not (Test-Path "node_modules")) {
      Write-Host "==> npm install (desktop/)"
      if (Test-Path "package-lock.json") { npm ci } else { npm install }
    }
  }

  if ($NoHmr) {
    # Packaged shape: the window loads the sidecar's static web/dist.
    Remove-Item Env:\LITECODE_UI_DEV_URL -ErrorAction SilentlyContinue
    Remove-Item Env:\LITECODE_DEV_UPSTREAM_FILE -ErrorAction SilentlyContinue
    Write-Host "==> static UI (sidecar web/dist). Omit -NoHmr for Vite hot reload."
  } else {
    $vite = Start-DevVite -RepoRoot $Root -Port $WebPort -SkipInstall ([bool]$SkipNpmInstall)
  }

  # Electron must spawn the sidecar itself. LITECODE_DEV_URL would attach to a
  # remote serve and skip that. LITECODE_UI_DEV_URL only replaces the document.
  Remove-Item Env:\LITECODE_DEV_URL -ErrorAction SilentlyContinue
  $env:LITECODE_SIDECAR_DIR = $Product

  $uiLine = if ($NoHmr) { "static web/dist" } else { $env:LITECODE_UI_DEV_URL }
  Write-Host @"

==> starting Electron desktop shell
    sidecar: $Product
    ui:      $uiLine
    profile: $Profile$(if ($Cuda) { "`n    cuda:    ort-cuda (tag only if CUDA EP actually opens)" } else { "" })
    (close the window to stop; sidecar exits with the host)

"@
  npm run dev
} finally {
  Stop-DevProcess $vite
  Restore-EnvValue "LITECODE_DEV_URL" $savedDevUrl
  Restore-EnvValue "LITECODE_UI_DEV_URL" $savedUiDevUrl
  Restore-EnvValue "LITECODE_DEV_UPSTREAM_FILE" $savedUpstreamFile
  Restore-EnvValue "LITECODE_SIDECAR_DIR" $savedSidecarDir
  Pop-Location
}
