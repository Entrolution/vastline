# install.ps1 — download the latest vastline release binary and wire it into Claude Code.
#
#   irm https://raw.githubusercontent.com/Entrolution/vastline/main/install.ps1 | iex
#
# Override the install dir with $env:VASTLINE_BIN_DIR (default: %LOCALAPPDATA%\vastline).

$ErrorActionPreference = "Stop"

$repo = "Entrolution/vastline"
$dir = if ($env:VASTLINE_BIN_DIR) { $env:VASTLINE_BIN_DIR } else { Join-Path $env:LOCALAPPDATA "vastline" }
$target = if ($env:PROCESSOR_ARCHITECTURE -eq "ARM64") { "aarch64-pc-windows-msvc" } else { "x86_64-pc-windows-msvc" }
$asset = "vastline-$target.zip"
$url = "https://github.com/$repo/releases/latest/download/$asset"

New-Item -ItemType Directory -Force -Path $dir | Out-Null
$zip = Join-Path $env:TEMP "vastline-$target.zip"
Write-Host "downloading $url"
Invoke-WebRequest -Uri $url -OutFile $zip
Expand-Archive -Path $zip -DestinationPath $dir -Force
Remove-Item $zip -ErrorAction SilentlyContinue

$exe = Join-Path $dir "vastline.exe"
Write-Host "installed -> $exe"
Write-Host ""

# Wire it into %USERPROFILE%\.claude\settings.json (backs up first, captures any existing line).
& $exe install

# Note if the install dir isn't on PATH, so bare `vastline ...` commands resolve.
$onPath = ($env:Path -split ';') -contains $dir
if (-not $onPath) {
    Write-Host ""
    Write-Host "note: $dir is not on your PATH — add it to run 'vastline' directly,"
    Write-Host "      or use the full path: $exe"
}

Write-Host ""
Write-Host "next: add a read-only API key —"
Write-Host "  vastai create api-key --name vastline --permissions '{\"api\": {\"instance_read\": {}, \"user_read\": {}}}'"
Write-Host "  $exe key set"
