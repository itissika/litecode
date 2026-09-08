# Assemble a cwd-independent product tree for smoke / Electron extraResources.
# Does not build Electron.
param(
  [string]$OutDir = "",
  [string]$Profile = "release",
  [switch]$SkipWeb,
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

$ModelDir = Join-Path $Root "models\ibm-granite\granite-embedding-97m-multilingual-r2"
if (-not $SkipModel) {
  $Bundle = Join-Path $Root "scripts\bundle_embed_model.sh"
  if (Test-Path $Bundle) {
    Write-Host "==> bundling embed model (bash)"
    bash $Bundle
  }
  if (-not (Test-Path $ModelDir)) {
    Write-Warning "models/ bundle missing at $ModelDir — packaging may be incomplete"
  }
} else {
  Write-Host "==> skipping embed model (-SkipModel, slim SKU)"
}

if ($env:LITECODE_CHANNEL -ne "official") {
  $env:LITECODE_CHANNEL = "nightly"
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

if (-not $SkipModel -and (Test-Path (Join-Path $Root "models"))) {
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
  models\       embedding weights when bundled (full SKU)
  *.dll         native runtime deps when present

Global settings DB: %LOCALAPPDATA%\litecode\
Per-workspace data: <workspace>\.litecode\
"@ | Set-Content -Encoding utf8 (Join-Path $OutDir "README.txt")

Write-Host "==> done: $OutDir"
Get-ChildItem $OutDir | Format-Table Name, Length
