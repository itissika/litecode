# Assemble a cwd-independent product tree for smoke / Electron extraResources.
# Does not build Electron.
param(
  [string]$OutDir = "",
  [string]$Profile = "release",
  [switch]$SkipWeb,
  # Default is slim: no embed model in the product tree (downloaded at runtime).
  # -WithModel copies models/ in (local offline builds, package_win.ps1 -Full).
  [switch]$WithModel,
  # Deprecated: slim is the default; kept so older callers keep working.
  [switch]$SkipModel,
  [string]$TargetTriple = "",
  [string]$Features = "",
  [string]$TargetDir = "",
  [switch]$KeepCudaDylibs
)

$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
if (-not $OutDir) { $OutDir = Join-Path $Root "dist\product" }

$ProfileDir = if ($Profile -eq "debug") { "debug" } else { "release" }
$CargoArgs = @("build")
if ($Profile -ne "debug") { $CargoArgs += "--release" }
if ($TargetTriple) { $CargoArgs += @("--target", $TargetTriple) }
if ($Features) { $CargoArgs += @("--features", $Features) }
if ($TargetDir) { $CargoArgs += @("--target-dir", $TargetDir) }

# ISSUE-015: remap cargo registry/git paths in release panic/debuginfo only.
function Append-RustflagRemap {
  param([Parameter(Mandatory = $true)][string]$Flag)
  $cur = if ($null -ne $env:RUSTFLAGS) { $env:RUSTFLAGS } else { "" }
  $tokens = @()
  if ($cur.Trim().Length -gt 0) {
    $tokens = $cur -split '\s+' | Where-Object { $_ -ne "" }
  }
  if ($tokens -contains $Flag) { return }
  if ($cur.Trim().Length -gt 0) {
    $env:RUSTFLAGS = "$($cur.Trim()) $Flag"
  } else {
    $env:RUSTFLAGS = $Flag
  }
}

function Ensure-ReleaseRustflagsRemaps {
  if ($Profile -eq "debug") { return }
  $cargoHome = if ($env:CARGO_HOME -and $env:CARGO_HOME.Trim().Length -gt 0) {
    $env:CARGO_HOME
  } else {
    Join-Path $env:USERPROFILE ".cargo"
  }
  if (Test-Path -LiteralPath $cargoHome) {
    $cargoHome = (Resolve-Path -LiteralPath $cargoHome).Path
  }
  # Use the resolved Windows path form cargo/rustc typically emit (backslash).
  Append-RustflagRemap "--remap-path-prefix=$cargoHome\registry=/rustc/registry"
  Append-RustflagRemap "--remap-path-prefix=$cargoHome\git=/rustc/git"
}

Write-Host "==> product root: $OutDir (profile=$Profile)"
if (Test-Path $OutDir) { Remove-Item -Recurse -Force $OutDir }
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

if (-not $SkipWeb) {
  Write-Host "==> building web/dist"
  Push-Location (Join-Path $Root "web")
  try {
    if (Test-Path "package-lock.json") { npm ci } else { npm install }
    npm run build
  } finally { Pop-Location }
}

$WebDist = Join-Path $Root "web\dist"
if (-not (Test-Path $WebDist)) {
  throw "web/dist missing; run npm run build in web/ or omit -SkipWeb"
}

$IncludeModel = [bool]$WithModel -and -not $SkipModel
$ModelDir = Join-Path $Root "models\ibm-granite\granite-embedding-97m-multilingual-r2"
if ($IncludeModel) {
  $Bundle = Join-Path $Root "scripts\bundle_embed_model.sh"
  if (Test-Path $Bundle) {
    Write-Host "==> bundling embed model (bash)"
    bash $Bundle
  }
  if (-not (Test-Path $ModelDir)) {
    Write-Warning "models/ bundle missing at $ModelDir — packaging may be incomplete"
  }
} else {
  Write-Host "==> slim product: embed model not bundled (downloaded on demand; pass -WithModel to embed)"
}

if ($env:LITECODE_CHANNEL -ne "official") {
  $env:LITECODE_CHANNEL = "nightly"
}

Ensure-ReleaseRustflagsRemaps
if ($Profile -ne "debug") {
  Write-Host "==> RUSTFLAGS (release remap): $($env:RUSTFLAGS)"
}
Write-Host "==> cargo $($CargoArgs -join ' ') (LITECODE_CHANNEL=$($env:LITECODE_CHANNEL))"
Push-Location $Root
$prevEa = $ErrorActionPreference
$ErrorActionPreference = "Continue"
try {
  $cargoOut = & cargo @CargoArgs 2>&1
  $cargoOut | ForEach-Object { Write-Host "$_" }
  if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE" }
} finally {
  $ErrorActionPreference = $prevEa
  Pop-Location
}

$BinDir = if ($TargetDir) {
  Join-Path $TargetDir $ProfileDir
} elseif ($TargetTriple) {
  Join-Path $Root "target\$TargetTriple\$ProfileDir"
} else {
  Join-Path $Root "target\$ProfileDir"
}

$Exe = Join-Path $BinDir "litecode.exe"
if (-not (Test-Path $Exe)) { $Exe = Join-Path $BinDir "litecode" }
if (-not (Test-Path $Exe)) { throw "missing litecode binary under $BinDir" }

Copy-Item -Force $Exe $OutDir
$dlls = Get-ChildItem $BinDir -Filter *.dll -ErrorAction SilentlyContinue
if (-not $KeepCudaDylibs) {
  $dlls = $dlls | Where-Object { $_.Name -notmatch '(?i)(providers_cuda|cudnn|cublas|cudart|nvrtc|cufft)' }
}
$dlls | Copy-Item -Force -Destination $OutDir

if ($KeepCudaDylibs) {
  $cudaDll = Join-Path $OutDir "onnxruntime_providers_cuda.dll"
  if (-not (Test-Path $cudaDll)) {
    throw "CUDA EP dll missing after assemble: $cudaDll — cargo --features ort-cuda did not emit it next to the binary"
  }
}

New-Item -ItemType Directory -Force -Path (Join-Path $OutDir "web") | Out-Null
Copy-Item -Recurse -Force $WebDist (Join-Path $OutDir "web\dist")

if ($IncludeModel -and (Test-Path (Join-Path $Root "models"))) {
  New-Item -ItemType Directory -Force -Path (Join-Path $OutDir "models") | Out-Null
  Copy-Item -Recurse -Force (Join-Path $Root "models\*") (Join-Path $OutDir "models\")
}

$BinName = Split-Path $Exe -Leaf
@"
Litecode product layout (sidecar-ready)

  .\$BinName serve --bind 127.0.0.1:0 --require-auth
  # set LITECODE_TOKEN in the environment (host-injected; users never type it)

Layout:
  $BinName      kernel binary
  web\dist\     UI (served by litecode)
  models\       embedding weights only with -WithModel (slim default downloads
                them to %LOCALAPPDATA%\litecode\bundles\models on first use)
  *.dll         native runtime deps when present

Global settings DB: %LOCALAPPDATA%\litecode\
Per-workspace data: <workspace>\.litecode\
"@ | Set-Content -Encoding utf8 (Join-Path $OutDir "README.txt")

Write-Host "==> done: $OutDir"
Get-ChildItem $OutDir | Format-Table Name, Length
