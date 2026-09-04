# Build Windows NSIS installer (and portable unless -SkipPortable).
# Unsigned unless CSC_* env is set.
param(
  [switch]$SkipAssemble,
  [switch]$SkipWeb,
  [switch]$SkipModel,
  [switch]$SkipLinuxBundle,
  [switch]$SkipPortable,
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

# GitHub checkout mtimes are "now"; the Linux artifact is older even at the same SHA.
if ($env:GITHUB_ACTIONS) {
  $linuxDir = Join-Path $Root "dist\linux"
  if (Test-Path -LiteralPath $linuxDir) {
    Get-ChildItem -LiteralPath $linuxDir -File -ErrorAction SilentlyContinue |
      ForEach-Object { $_.LastWriteTime = Get-Date }
  }
}

if ($SkipLinuxBundle) {
  Write-Host "==> skipping Linux bundle (slim SKU); Open Remote reads LITECODE_BUNDLE_ROOT / %LOCALAPPDATA%\litecode\bundles"
} else {
  $null = & (Join-Path $Root "scripts\ensure_linux_bundle.ps1") -Root $Root -Require
}

Push-Location (Join-Path $Root "desktop")
$builderConfig = $null
try {
  if (-not (Test-Path "node_modules")) { npm ci }
  $winArgs = if ($SkipPortable) { @("--win", "nsis", "--x64") } else { @("--win", "--x64") }
  if ($SkipLinuxBundle) {
    npm run build
    $pkg = Get-Content -Raw -LiteralPath "package.json" | ConvertFrom-Json
    $build = $pkg.build
    $filtered = @()
    foreach ($item in $build.extraResources) {
      $from = [string]$item.from
      if ($from -match 'dist[/\\]linux') { continue }
      $filtered += $item
    }
    $build.extraResources = @($filtered)
    $builderConfig = Join-Path $env:TEMP ("litecode-electron-builder-slim-" + [guid]::NewGuid().ToString("N") + ".json")
    $json = $build | ConvertTo-Json -Depth 16
    [System.IO.File]::WriteAllText($builderConfig, $json)
    npx electron-builder @winArgs --config $builderConfig
  } elseif ($SkipPortable) {
    npm run build
    npx electron-builder @winArgs
  } else {
    npm run dist:win
  }
} finally {
  if ($builderConfig -and (Test-Path -LiteralPath $builderConfig)) {
    Remove-Item -LiteralPath $builderConfig -Force -ErrorAction SilentlyContinue
  }
  Pop-Location
}

Write-Host "==> artifacts:"
Get-ChildItem (Join-Path $Root "desktop\out") -File | Format-Table Name, Length
