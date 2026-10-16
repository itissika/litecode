# Shared PowerShell helpers (dot-source: . (Join-Path $PSScriptRoot "lib\common.ps1")).

function Test-Command([string]$Name) {
  return [bool](Get-Command $Name -ErrorAction SilentlyContinue)
}

# Throw if the last native command exited non-zero.
function Assert-LastExitCode([string]$What) {
  if ($null -ne $LASTEXITCODE -and $LASTEXITCODE -ne 0) {
    throw "$What failed with exit code $LASTEXITCODE"
  }
}
