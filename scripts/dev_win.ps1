# Windows native Electron desktop dev loop (end-state shell).
#
# Spawns the same shape as a packaged install: Electron host + litecode sidecar
# (serve --bind 127.0.0.1:0 --require-auth, token injected). UI is the built
# web/dist served by the sidecar — not the WSL/browser Vite loop (scripts/serve.sh).
#
# Usage (from repo root, PowerShell):
#   ./scripts/dev_win.ps1
#   ./scripts/dev_win.ps1 -RebuildWeb          # force npm run build in web/
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
  [switch]$Cuda
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
    Write-Host "==> reusing web/dist (pass -RebuildWeb to rebuild UI)"
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

Push-Location $Desktop
try {
  if (-not $SkipNpmInstall) {
    if (-not (Test-Path "node_modules")) {
      Write-Host "==> npm install (desktop/)"
      if (Test-Path "package-lock.json") { npm ci } else { npm install }
    }
  }

  # End-state path: Electron must spawn the sidecar itself (not attach to Vite/API).
  Remove-Item Env:\LITECODE_DEV_URL -ErrorAction SilentlyContinue
  $env:LITECODE_SIDECAR_DIR = $Product

  Write-Host @"

==> starting Electron desktop shell
    sidecar: $Product
    profile: $Profile$(if ($Cuda) { "`n    cuda:    ort-cuda (tag only if CUDA EP actually opens)" } else { "" })
    (close the window to stop; sidecar exits with the host)

"@
  npm run dev
} finally {
  Pop-Location
}
