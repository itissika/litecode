# Shared product version + Linux bundle path helpers (dot-source from other scripts).
# Canonical version: [package].version in Cargo.toml.

function Get-LitecodeProductVersion {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Root
    )
    $cargoToml = Join-Path $Root "Cargo.toml"
    if (-not (Test-Path -LiteralPath $cargoToml)) {
        throw "Cargo.toml not found at $cargoToml"
    }
    foreach ($line in Get-Content -LiteralPath $cargoToml) {
        if ($line -match '^\s*version\s*=\s*"([^"]+)"\s*$') {
            return $Matches[1]
        }
    }
    throw "Could not read [package].version from Cargo.toml"
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
    $paths = @(
        (Join-Path $Root "Cargo.toml"),
        (Join-Path $Root "web\dist\index.html")
    )
    $srcDir = Join-Path $Root "src"
    if (Test-Path -LiteralPath $srcDir) {
        $paths += Get-ChildItem -LiteralPath $srcDir -Recurse -File -ErrorAction SilentlyContinue |
            Select-Object -ExpandProperty FullName
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
