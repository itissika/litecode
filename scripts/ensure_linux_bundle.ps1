# Verify the Linux server tar exists (and optionally warn if stale vs src/web).
# Dot-sources scripts/product_version.ps1.

param(
    [string]$Root = "",
    [switch]$Require,
    [switch]$WarnOnly
)

$ErrorActionPreference = "Stop"
if (-not $Root) {
    $Root = Resolve-Path (Join-Path $PSScriptRoot "..")
}

. (Join-Path $PSScriptRoot "product_version.ps1")

$layout = Get-LinuxBundleLayout -Root $Root
$tar = $layout.StableTar
$checksum = $layout.StableChecksum
$version = $layout.Version

if (-not (Test-Path -LiteralPath $tar) -or -not (Test-Path -LiteralPath $checksum)) {
    $versionedTar = $layout.VersionedTar
    $versionedChecksum = $layout.VersionedChecksum
    if (
        (Test-Path -LiteralPath $versionedTar) -and
        (Test-Path -LiteralPath $versionedChecksum)
    ) {
        New-Item -ItemType Directory -Force -Path $layout.LinuxDir | Out-Null
        Copy-Item -LiteralPath $versionedTar -Destination $tar -Force
        Copy-Item -LiteralPath $versionedChecksum -Destination $checksum -Force
        Write-Warning @"
Staged Linux bundle from versioned dist path:
  $versionedTar
  -> $tar
Rebuild in WSL when convenient: ./scripts/package_linux.sh
"@
    } else {
        $msg = @"
Linux server bundle missing for version $version.

  Expected:
    $tar
    $checksum

  Build in WSL (same git commit as Windows):
    ./scripts/package_linux.sh

  Pure-local dev (no Open Remote): pass -SkipLinuxBundle to dev_win.ps1
"@
        if ($Require) {
            throw $msg
        }
        Write-Warning $msg.Trim()
        return $layout
    }
}

if (Test-LinuxBundleStale -Root $Root -TarPath $tar) {
    $warn = @"
Linux server bundle looks older than src/, web/src, or Cargo.toml.

  $tar

  Rebuild in WSL so Open Remote matches the Windows sidecar:
    ./scripts/package_linux.sh
"@
    if ($WarnOnly -or -not $Require) {
        Write-Warning $warn.Trim()
    } else {
        throw $warn.Trim()
    }
}

Write-Host "==> Linux bundle OK (v$version): $tar"
return $layout
