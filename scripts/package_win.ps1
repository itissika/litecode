# Build Windows NSIS installer (and portable unless -SkipPortable).
# Unsigned unless CSC_* env is set.
param(
  [switch]$SkipAssemble,
  [switch]$SkipWeb,
  [switch]$SkipModel,
  [switch]$SkipLinuxBundle,
  [switch]$LinuxBundleWarnOnly,
  [switch]$SkipLinuxFreshness,
  [switch]$SkipPortable,
  [string]$Profile = "release",
  [string]$ArtifactInfix = ""
)

$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
. (Join-Path $PSScriptRoot "product_version.ps1")
$Version = Get-LitecodeProductVersion -Root $Root

function Assert-LastExitCode([string]$What) {
  if ($null -ne $LASTEXITCODE -and $LASTEXITCODE -ne 0) {
    throw "$What failed with exit code $LASTEXITCODE"
  }
}

function Invoke-ElectronBuilder {
  param([string[]]$BuilderArgs)
  $maxAttempts = 3
  for ($attempt = 1; $attempt -le $maxAttempts; $attempt++) {
    Write-Host "==> electron-builder (attempt $attempt/$maxAttempts)"
    & npx electron-builder @BuilderArgs
    if ($LASTEXITCODE -eq 0) { return }
    $code = $LASTEXITCODE
    if ($attempt -eq $maxAttempts) {
      throw "electron-builder failed after $maxAttempts attempts (exit $code)"
    }
    Write-Warning "electron-builder failed with exit code $code; retrying"
    if (-not $env:GITHUB_ACTIONS -and -not $env:ELECTRON_MIRROR) {
      $env:ELECTRON_MIRROR = "https://npmmirror.com/mirrors/electron/"
      $env:ELECTRON_BUILDER_BINARIES_MIRROR = "https://npmmirror.com/mirrors/electron-builder-binaries/"
      Write-Host "==> retry via npmmirror ($($env:ELECTRON_MIRROR))"
    }
    Start-Sleep -Seconds (3 * $attempt)
  }
}

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
  $ensureArgs = @{ Root = $Root }
  if ($LinuxBundleWarnOnly) { $ensureArgs.WarnOnly = $true }
  else { $ensureArgs.Require = $true }
  if ($SkipLinuxFreshness) { $ensureArgs.SkipFreshness = $true }
  $null = & (Join-Path $Root "scripts\ensure_linux_bundle.ps1") @ensureArgs
}

if (-not $env:GITHUB_ACTIONS -and -not $env:ELECTRON_MIRROR) {
  $env:ELECTRON_MIRROR = "https://npmmirror.com/mirrors/electron/"
  Write-Host "==> ELECTRON_MIRROR=$($env:ELECTRON_MIRROR) (set ELECTRON_MIRROR to override)"
}
if (-not $env:GITHUB_ACTIONS -and -not $env:ELECTRON_BUILDER_BINARIES_MIRROR) {
  $env:ELECTRON_BUILDER_BINARIES_MIRROR = "https://npmmirror.com/mirrors/electron-builder-binaries/"
}

Push-Location (Join-Path $Root "desktop")
$builderConfig = $null
$infix = $ArtifactInfix.Trim().Trim("-")
try {
  if (-not (Test-Path "node_modules")) {
    npm ci
    Assert-LastExitCode "npm ci"
  }
  npm run build
  Assert-LastExitCode "desktop build (tsc)"

  $winArgs = if ($SkipPortable) { @("--win", "nsis", "--x64") } else { @("--win", "--x64") }
  $needConfig = [bool]$SkipLinuxBundle -or ($infix -ne "")
  if ($needConfig) {
    $pkg = Get-Content -Raw -LiteralPath "package.json" | ConvertFrom-Json
    $build = $pkg.build
    if ($SkipLinuxBundle) {
      $filtered = @()
      foreach ($item in $build.extraResources) {
        $from = [string]$item.from
        if ($from -match 'dist[/\\]linux') { continue }
        $filtered += $item
      }
      $build.extraResources = @($filtered)
    }
    if ($infix) {
      $build.win.artifactName = "Litecode-`${version}-$infix-`${os}-`${arch}.`${ext}"
      $build.nsis.artifactName = "Litecode-Setup-`${version}-$infix-`${arch}.`${ext}"
      $build.portable.artifactName = "Litecode-Portable-`${version}-$infix-`${arch}.`${ext}"
    }
    $builderConfig = Join-Path $env:TEMP ("litecode-electron-builder-" + [guid]::NewGuid().ToString("N") + ".json")
    $json = $build | ConvertTo-Json -Depth 16
    [System.IO.File]::WriteAllText($builderConfig, $json)
    $winArgs += @("--config", $builderConfig)
  }
  Invoke-ElectronBuilder $winArgs
} finally {
  if ($builderConfig -and (Test-Path -LiteralPath $builderConfig)) {
    Remove-Item -LiteralPath $builderConfig -Force -ErrorAction SilentlyContinue
  }
  Pop-Location
}

$outDir = Join-Path $Root "desktop\out"
$infixPart = if ($infix) { "-$infix" } else { "" }
$setup = Join-Path $outDir "Litecode-Setup-$Version$infixPart-x64.exe"
if (-not (Test-Path -LiteralPath $setup)) {
  throw "expected installer missing: $setup"
}
if (-not $SkipPortable) {
  $portable = Join-Path $outDir "Litecode-Portable-$Version$infixPart-x64.exe"
  if (-not (Test-Path -LiteralPath $portable)) {
    throw "expected portable missing: $portable"
  }
}

Write-Host "==> artifacts:"
Get-ChildItem $outDir -File | Format-Table Name, Length
