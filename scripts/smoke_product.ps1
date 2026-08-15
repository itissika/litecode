# Smoke-test a product tree from a non-repo cwd (Windows).
param(
  [string]$ProductDir = ""
)

$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
if (-not $ProductDir) { $ProductDir = Join-Path $Root "dist\product" }

$Bin = Join-Path $ProductDir "litecode.exe"
if (-not (Test-Path $Bin)) { $Bin = Join-Path $ProductDir "litecode" }
if (-not (Test-Path $Bin)) { throw "missing litecode binary in $ProductDir — run assemble_product.ps1 first" }

$Token = "smoke-token-$(Get-Date -Format yyyyMMddHHmmss)"
$WsA = Join-Path $env:TEMP "litecode-ws-a-$([guid]::NewGuid().ToString('N'))"
$WsB = Join-Path $env:TEMP "litecode-ws-b-$([guid]::NewGuid().ToString('N'))"
$Log1 = Join-Path $env:TEMP "litecode-smoke-$([guid]::NewGuid().ToString('N')).log"
$Err1 = Join-Path $env:TEMP "litecode-smoke-$([guid]::NewGuid().ToString('N')).err"
$Log2 = Join-Path $env:TEMP "litecode-smoke2-$([guid]::NewGuid().ToString('N')).log"
$Err2 = Join-Path $env:TEMP "litecode-smoke2-$([guid]::NewGuid().ToString('N')).err"
$LogConflict = Join-Path $env:TEMP "litecode-smoke-conflict-$([guid]::NewGuid().ToString('N')).log"
$ErrConflict = Join-Path $env:TEMP "litecode-smoke-conflict-$([guid]::NewGuid().ToString('N')).err"
$P1 = $null
$P2 = $null
$PRestart = $null

function Wait-Ready([string]$LogPath, [string]$ErrPath, [int]$TimeoutSec = 90) {
  $deadline = (Get-Date).AddSeconds($TimeoutSec)
  while ((Get-Date) -lt $deadline) {
    foreach ($p in @($LogPath, $ErrPath)) {
      if (Test-Path $p) {
        $line = Select-String -Path $p -Pattern '^LITECODE_READY ' | Select-Object -First 1
        if ($line) { return $line.Line }
      }
    }
    Start-Sleep -Milliseconds 150
  }
  foreach ($p in @($LogPath, $ErrPath)) {
    if (Test-Path $p) { Write-Host "---- $p ----"; Get-Content $p | Write-Host }
  }
  throw "timeout waiting for LITECODE_READY"
}

function Invoke-Http([string]$Url, [hashtable]$Headers = @{}) {
  try {
    return Invoke-WebRequest -Uri $Url -Headers $Headers -UseBasicParsing -TimeoutSec 10
  } catch {
    $resp = $_.Exception.Response
    if ($resp) {
      return [pscustomobject]@{ StatusCode = [int]$resp.StatusCode; Content = "" }
    }
    throw
  }
}

try {
  New-Item -ItemType Directory -Force -Path $WsA, $WsB | Out-Null
  $env:LITECODE_TOKEN = $Token

  Write-Host "==> smoke from cwd=$env:TEMP using $Bin"
  $P1 = Start-Process -FilePath $Bin -ArgumentList @(
    "serve", "--bind", "127.0.0.1:0", "--require-auth", "--workspace", $WsA
  ) -WorkingDirectory $env:TEMP -RedirectStandardOutput $Log1 -RedirectStandardError $Err1 -PassThru -NoNewWindow

  $Ready = Wait-Ready $Log1 $Err1
  Write-Host "    $Ready"
  if ($Ready -notmatch 'LITECODE_READY (http://127\.0\.0\.1:\d+/?)') {
    throw "unparseable READY line: $Ready"
  }
  $Base = $Matches[1].TrimEnd('/')

  $health = Invoke-Http "$Base/health"
  if ($health.Content -notmatch '"ok"\s*:\s*true') { throw "health failed: $($health.Content)" }

  $ok = Invoke-Http "$Base/api/settings" @{ Authorization = "Bearer $Token" }
  if ([int]$ok.StatusCode -ne 200) { throw "expected 200 with token, got $($ok.StatusCode)" }

  $deny = Invoke-Http "$Base/api/settings"
  if ([int]$deny.StatusCode -ne 401) { throw "expected 401 without token, got $($deny.StatusCode)" }

  $P2 = Start-Process -FilePath $Bin -ArgumentList @(
    "serve", "--bind", "127.0.0.1:0", "--require-auth", "--workspace", $WsB
  ) -WorkingDirectory $env:TEMP -RedirectStandardOutput $Log2 -RedirectStandardError $Err2 -PassThru -NoNewWindow
  $null = Wait-Ready $Log2 $Err2

  $conflict = Start-Process -FilePath $Bin -ArgumentList @(
    "serve", "--bind", "127.0.0.1:0", "--require-auth", "--workspace", $WsA
  ) -WorkingDirectory $env:TEMP -RedirectStandardOutput $LogConflict -RedirectStandardError $ErrConflict -Wait -PassThru -NoNewWindow
  if ($conflict.ExitCode -eq 0) {
    Get-Content $LogConflict, $ErrConflict -ErrorAction SilentlyContinue | Write-Host
    throw "expected lock conflict on same workspace"
  }
  $conflictText = (@(Get-Content $LogConflict -Raw -ErrorAction SilentlyContinue) + @(Get-Content $ErrConflict -Raw -ErrorAction SilentlyContinue)) -join "`n"
  if ($conflictText -notmatch 'already open|lock busy') {
    Write-Host $conflictText
    throw "conflict output missing lock message"
  }

  # Process restart: stop A, then start A again (lock released on process exit).
  Write-Host "==> restart workspace A after stop (process-level relaunch)"
  Stop-Process -Id $P1.Id -Force -ErrorAction SilentlyContinue
  $P1.WaitForExit(15000) | Out-Null
  $P1 = $null
  $LogRestart = Join-Path $env:TEMP "litecode-smoke-restart-$([guid]::NewGuid().ToString('N')).log"
  $ErrRestart = Join-Path $env:TEMP "litecode-smoke-restart-$([guid]::NewGuid().ToString('N')).err"
  $PRestart = Start-Process -FilePath $Bin -ArgumentList @(
    "--workspace", $WsA, "serve", "--bind", "127.0.0.1:0", "--require-auth"
  ) -WorkingDirectory $WsA -RedirectStandardOutput $LogRestart -RedirectStandardError $ErrRestart -PassThru -NoNewWindow
  $Ready2 = Wait-Ready $LogRestart $ErrRestart
  Write-Host "    $Ready2"
  if ($Ready2 -notmatch 'LITECODE_READY (http://127\.0\.0\.1:\d+/?)') {
    throw "unparseable READY after restart: $Ready2"
  }
  $Base2 = $Matches[1].TrimEnd('/')
  if ($Base2 -eq $Base) {
    Write-Host "    note: same port reused after restart (ok)"
  } else {
    Write-Host "    port changed after restart ($Base -> $Base2)"
  }
  $openUri = "$Base2/api/workspace/" + "open"
  try {
    $openResp = Invoke-WebRequest -Uri $openUri -Method POST -Headers @{ Authorization = "Bearer $Token" } -ContentType "application/json" -Body '{"path":"C:/tmp"}' -UseBasicParsing -TimeoutSec 10
    $openStatus = [int]$openResp.StatusCode
  } catch {
    $openStatus = [int]$_.Exception.Response.StatusCode
  }
  if ($openStatus -ne 404 -and $openStatus -ne 405) {
    throw "expected workspace open HTTP route gone (404/405), got $openStatus"
  }

  Write-Host "==> smoke ok"
} finally {
  if ($P1 -and -not $P1.HasExited) { Stop-Process -Id $P1.Id -Force -ErrorAction SilentlyContinue }
  if ($P2 -and -not $P2.HasExited) { Stop-Process -Id $P2.Id -Force -ErrorAction SilentlyContinue }
  if ($PRestart -and -not $PRestart.HasExited) { Stop-Process -Id $PRestart.Id -Force -ErrorAction SilentlyContinue }
  Remove-Item -Recurse -Force $WsA, $WsB -ErrorAction SilentlyContinue
  Remove-Item -Force $Log1, $Err1, $Log2, $Err2, $LogConflict, $ErrConflict -ErrorAction SilentlyContinue
  Remove-Item -Force $LogRestart, $ErrRestart -ErrorAction SilentlyContinue
  Remove-Item Env:LITECODE_TOKEN -ErrorAction SilentlyContinue
}
