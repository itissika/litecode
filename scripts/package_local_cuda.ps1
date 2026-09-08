# Personal CUDA nightly Windows installer. Independent of package_local.ps1.
# Builds the sidecar with ORT CUDA EP, keeps CUDA dylibs, names the NSIS
# artifact Litecode-Setup-<version>-cuda-x64.exe. Does not rebuild the Linux tar.
#
# Usage (repo root, PowerShell):
#   ./scripts/package_local_cuda.ps1
#   ./scripts/package_local_cuda.ps1 -SkipLinuxBundle
#
# CUDA toolkit + cuDNN must already be on PATH (ORT CUDA EP). Uses cargo
# --target-dir target\cuda-accel so the CPU product tree stays untouched.

param(
  [string]$Profile = "release",
  [switch]$SkipLinuxBundle,
  [switch]$SkipWeb
)

$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
. (Join-Path $PSScriptRoot "product_version.ps1")
$Version = Get-LitecodeProductVersion -Root $Root

if ($env:LITECODE_CHANNEL -ne "official") {
  $env:LITECODE_CHANNEL = "nightly"
}
if (-not $env:ORT_CUDA_VERSION) {
  $env:ORT_CUDA_VERSION = "12"
}

$TargetDir = Join-Path $Root "target\cuda-accel"
$linuxTar = Join-Path $Root "dist\linux\litecode-server-linux-x64.tar.gz"
$hasLinux = Test-Path -LiteralPath $linuxTar
$skipLinux = [bool]$SkipLinuxBundle -or -not $hasLinux

if ($skipLinux -and -not $SkipLinuxBundle -and -not $hasLinux) {
  Write-Warning "Linux tar missing at $linuxTar — CUDA installer will omit the Open Remote bundle (tar is not rebuilt)"
} elseif (-not $skipLinux) {
  Write-Host "==> reusing existing Linux tar (not rebuilt): $linuxTar"
}

Write-Host "==> CUDA nightly assemble (ort-cuda, KeepCudaDylibs, v$Version, ORT_CUDA_VERSION=$($env:ORT_CUDA_VERSION))"
$assembleArgs = @{
  Profile         = $Profile
  Features        = "ort-cuda"
  TargetDir       = $TargetDir
  KeepCudaDylibs  = $true
}
if ($SkipWeb) { $assembleArgs.SkipWeb = $true }
& (Join-Path $Root "scripts\assemble_product.ps1") @assembleArgs

$ModelDir = Join-Path $Root "models\ibm-granite\granite-embedding-97m-multilingual-r2"
if (-not (Test-Path (Join-Path $ModelDir "artifacts\ort-lin-q8-emb-q4-bs128-a1.onnx"))) {
  throw "embed weights missing at $ModelDir — cannot build the product SKU"
}

$cudaDll = Join-Path $Root "dist\product\onnxruntime_providers_cuda.dll"
if (-not (Test-Path $cudaDll)) {
  throw "CUDA EP dll missing at $cudaDll after assemble — refusing to package a CPU-only tree as cuda"
}

Write-Host "==> Windows NSIS (CUDA sidecar, ArtifactInfix=cuda, v$Version)"
$packArgs = @{
  SkipAssemble   = $true
  Profile        = $Profile
  SkipPortable   = $true
  ArtifactInfix  = "cuda"
}
if ($skipLinux) { $packArgs.SkipLinuxBundle = $true }
else { $packArgs.LinuxBundleWarnOnly = $true }
& (Join-Path $Root "scripts\package_win.ps1") @packArgs

$outDir = Join-Path $Root "desktop\out"
$setup = Join-Path $outDir "Litecode-Setup-$Version-cuda-x64.exe"
if (-not (Test-Path -LiteralPath $setup)) {
  throw "expected CUDA installer missing: $setup"
}

Write-Host @"

==> CUDA nightly installer (v$Version, LITECODE_CHANNEL=nightly)
  Windows NSIS: $setup
  Sidecar:      $(Join-Path $Root "dist\product")  (includes onnxruntime_providers_cuda.dll)
  Linux tar:    $(if ($skipLinux) { "omitted (not rebuilt)" } else { $linuxTar })

Install this locally. Semantic retrieval shows a cuda tag only when the worker
actually opened CUDA EP (falls back to CPU EP otherwise).

"@
