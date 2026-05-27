# Download local-only assets that are intentionally not committed.
# Run from anywhere: powershell -ExecutionPolicy Bypass -File assets\fetch.ps1

$ErrorActionPreference = 'Stop'

# Fluent UI System Icons — pinned to a commit so every developer ends up with
# a byte-identical TTF. Bump these values to upgrade to a newer icon set.
$fluentRepo   = 'microsoft/fluentui-system-icons'
$fluentCommit = 'c154201b6194639bb2a2fb09292190b32da1bae3'
$fluentFile   = 'fonts/FluentSystemIcons-Regular.ttf'

$assetsDir = $PSScriptRoot
$fontsDir  = Join-Path $assetsDir 'fonts'
$dest      = Join-Path $fontsDir 'FluentSystemIcons-Regular.ttf'

if (Test-Path $dest) {
    Write-Host "Already present: $dest"
    exit 0
}

New-Item -ItemType Directory -Force -Path $fontsDir | Out-Null

$url = "https://raw.githubusercontent.com/$fluentRepo/$fluentCommit/$fluentFile"
Write-Host "Downloading $url"
Invoke-WebRequest -Uri $url -OutFile $dest -UseBasicParsing
Write-Host "Saved to $dest"
