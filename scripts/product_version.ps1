# Shared product version + Linux bundle path helpers (dot-source from other scripts).
# Canonical version: [package].version in Cargo.toml, read by
# `node scripts/build_components.mjs --print-version` (also used by package_linux.sh).

function Get-LitecodeProductVersion {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Root
    )
    $script = Join-Path $Root "scripts\build_components.mjs"
    $out = & node $script --print-version
    if ($LASTEXITCODE -ne 0) {
        throw "build_components.mjs --print-version failed with exit code $LASTEXITCODE"
    }
    $version = "$out".Trim()
    if (-not $version) {
        throw "Could not read [package].version from Cargo.toml"
    }
    return $version
}

function Get-LinuxBundleLayout {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Root
    )
    $version = Get-LitecodeProductVersion -Root $Root
    $linuxDir = Join-Path $Root "dist\linux"
    $stableTar = Join-Path $linuxDir "litecode-server-linux-x64.tar.gz"
    return @{
        Version           = $version
        LinuxDir          = $linuxDir
        StableTar         = $stableTar
        StableChecksum    = "$stableTar.sha256"
        VersionedTar      = Join-Path $Root "dist\litecode-server-$version-linux-x64.tar.gz"
        VersionedChecksum = Join-Path $Root "dist\litecode-server-$version-linux-x64.tar.gz.sha256"
    }
}

function Get-LinuxBundleFreshnessSources {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Root
    )
    # Do not use web/dist: assemble_product rebuilds it immediately before this
    # check, so its mtime is always newer than a same-SHA Linux tar (CI and local).
    $paths = @(
        (Join-Path $Root "Cargo.toml"),
        (Join-Path $Root "web\package.json")
    )
    foreach ($dir in @("src", "web\src")) {
        $full = Join-Path $Root $dir
        if (Test-Path -LiteralPath $full) {
            $paths += Get-ChildItem -LiteralPath $full -Recurse -File -ErrorAction SilentlyContinue |
                Select-Object -ExpandProperty FullName
        }
    }
    return $paths | Where-Object { Test-Path -LiteralPath $_ }
}

function Test-LinuxBundleStale {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Root,
        [Parameter(Mandatory = $true)]
        [string]$TarPath
    )
    if (-not (Test-Path -LiteralPath $TarPath)) {
        return $false
    }
    $tarTime = (Get-Item -LiteralPath $TarPath).LastWriteTimeUtc
    $sources = Get-LinuxBundleFreshnessSources -Root $Root
    foreach ($source in $sources) {
        $sourceTime = (Get-Item -LiteralPath $source).LastWriteTimeUtc
        if ($sourceTime -gt $tarTime) {
            return $true
        }
    }
    return $false
}
