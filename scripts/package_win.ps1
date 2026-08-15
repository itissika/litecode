# Build Windows portable + NSIS installers (unsigned unless CSC_* env is set).
param(
  [switch]$SkipAssemble,
  [switch]$SkipWeb,
  [switch]$SkipModel,
  [string]$Profile = "release"
)

$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..")

if (-not $SkipAssemble) {
  $assembleArgs = @{ Profile = $Profile }
  if ($SkipWeb) { $assembleArgs.SkipWeb = $true }
  if ($SkipModel) { $assembleArgs.SkipModel = $true }
  & (Join-Path $Root "scripts\assemble_product.ps1") @assembleArgs
}

$Product = Join-Path $Root "dist\product"
$Exe = Join-Path $Product "litecode.exe"
if (-not (Test-Path $Exe)) {
  throw "sidecar missing at $Exe — assemble first"
}

$null = & (Join-Path $Root "scripts\ensure_linux_bundle.ps1") -Root $Root -Require
Push-Location (Join-Path $Root "desktop")
try {
  if (-not (Test-Path "node_modules")) { npm ci }
  npm run dist:win
} finally {
  Pop-Location
}

Write-Host "==> artifacts:"
Get-ChildItem (Join-Path $Root "desktop\out") -File | Format-Table Name, Length
