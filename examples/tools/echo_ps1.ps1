# Custom tool: echo a message from stdin JSON.
# Register with command: powershell  args: -NoProfile -File <path-to-this-script>
$ErrorActionPreference = "Stop"
$raw = [Console]::In.ReadToEnd()
try {
    $data = $raw | ConvertFrom-Json
} catch {
    [Console]::Error.WriteLine("invalid json input: $_")
    exit 1
}
if (-not $data.message -or $data.message -isnot [string]) {
    [Console]::Error.WriteLine("missing required string field: message")
    exit 1
}
Write-Output $data.message
exit 0
