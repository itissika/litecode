# Local nightly slim SKU: Windows installers without models/linux tar + WSL slim server tar.
# Does not publish a GitHub Release. Default CI packaging is unchanged.
#
# Usage (repo root, PowerShell):
#   ./scripts/package_local.ps1
#   ./scripts/package_local.ps1 -WslRoot /home/you/litecode
#
# Default -WslRoot maps this Windows checkout to /mnt/<drive>/...
# WSL checkout must be the same commit / version as this tree.

param(
  [string]$WslRoot = "",
  [string]$Profile = "release"
)

$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
. (Join-Path $PSScriptRoot "product_version.ps1")
$Version = Get-LitecodeProductVersion -Root $Root

function ConvertTo-WslPath([string]$WinPath) {
  $full = [System.IO.Path]::GetFullPath($WinPath)
  if ($full -match '^([A-Za-z]):\\(.*)$') {
    $drive = $Matches[1].ToLowerInvariant()
    $rest = ($Matches[2] -replace '\\', '/').TrimEnd('/')
    if ($rest) { return "/mnt/$drive/$rest" }
    return "/mnt/$drive"
  }
  throw "cannot map Windows path to WSL: $full"
}

function Invoke-WslBash([string]$Script) {
  & wsl.exe -e bash -lc $Script
  if ($LASTEXITCODE -ne 0) {
    throw "WSL command failed with exit code $LASTEXITCODE"
  }
}

if (-not $WslRoot) {
  $WslRoot = ConvertTo-WslPath $Root
  Write-Host "==> WSL root (mapped): $WslRoot"
} else {
  Write-Host "==> WSL root (override): $WslRoot"
}

$wslRootQ = $WslRoot.Replace("'", "'\''")
$winDist = Join-Path $Root "dist"
$wslWinDistQ = (ConvertTo-WslPath $winDist).Replace("'", "'\''")
$mappedRoot = (ConvertTo-WslPath $Root).TrimEnd('/')
$sharedCheckout = ($WslRoot.TrimEnd('/') -eq $mappedRoot)
$buildWeb = "1"
$skipWebAssemble = $false

if ($sharedCheckout) {
  # /mnt shares Windows node_modules (no @rollup/rollup-linux-*). Build UI on Windows.
  Write-Host "==> building web/dist on Windows (shared WSL checkout)"
  Push-Location (Join-Path $Root "web")
  try {
    if (-not (Test-Path "node_modules")) {
      if (Test-Path "package-lock.json") { npm ci } else { npm install }
    }
    npm run build
  } finally {
    Pop-Location
  }
  $buildWeb = "0"
  $skipWebAssemble = $true
}

Write-Host "==> WSL slim Linux tar (LITECODE_BUNDLE_MODEL=0 LITECODE_CHANNEL=nightly v$Version)"
$copyBack = if ($sharedCheckout) { "0" } else { "1" }
Invoke-WslBash @"
set -euo pipefail
cd '$wslRootQ'
export LITECODE_CHANNEL=nightly
export LITECODE_BUNDLE_MODEL=0
export LITECODE_BUILD_WEB=$buildWeb
./scripts/package_linux.sh
if [ "$copyBack" = "1" ]; then
  mkdir -p '$wslWinDistQ/linux'
  cp -f dist/linux/litecode-server-linux-x64.tar.gz '$wslWinDistQ/linux/'
  cp -f dist/linux/litecode-server-linux-x64.tar.gz.sha256 '$wslWinDistQ/linux/'
  if [ -f dist/litecode-server-$Version-linux-x64.tar.gz ]; then
    cp -f dist/litecode-server-$Version-linux-x64.tar.gz '$wslWinDistQ/'
    cp -f dist/litecode-server-$Version-linux-x64.tar.gz.sha256 '$wslWinDistQ/'
  fi
fi
"@

if ($env:LITECODE_CHANNEL -ne "official") {
  $env:LITECODE_CHANNEL = "nightly"
}

Write-Host "==> Windows slim assemble + installers (v$Version)"
$assembleArgs = @{
  Profile = $Profile
  SkipModel = $true
}
if ($skipWebAssemble) { $assembleArgs.SkipWeb = $true }
& (Join-Path $Root "scripts\assemble_product.ps1") @assembleArgs
& (Join-Path $Root "scripts\package_win.ps1") -SkipAssemble -SkipModel -SkipLinuxBundle -Profile $Profile

$outDir = Join-Path $Root "desktop\out"
$portable = Get-ChildItem -LiteralPath $outDir -Filter "Litecode-Portable-*.exe" -ErrorAction SilentlyContinue |
  Sort-Object LastWriteTime -Descending |
  Select-Object -First 1
if ($portable) {
  Copy-Item -Force -LiteralPath $portable.FullName -Destination (Join-Path $winDist $portable.Name)
}

Write-Host @"

==> nightly slim artifacts (v$Version)
  Linux tar:  $(Join-Path $winDist "litecode-server-$Version-linux-x64.tar.gz")
  Linux staged: $(Join-Path $winDist "linux\litecode-server-linux-x64.tar.gz")
  Windows:    $outDir
  Portable copy: $(if ($portable) { Join-Path $winDist $portable.Name } else { "(not found)" })

Embed weights are not in either package. Point the app at:
  LITECODE_MODEL_DIR  or  LITECODE_BUNDLE_ROOT\models\
  default %LOCALAPPDATA%\litecode\bundles\models\

Open Remote tar:
  LITECODE_LINUX_BUNDLE  or  LITECODE_BUNDLE_ROOT\linux\
  default %LOCALAPPDATA%\litecode\bundles\linux\

"@
