#Requires -Version 5.1
<#
.SYNOPSIS
    Builds Markmap HTML files from .mmd sources in .doc/markmap/
.DESCRIPTION
    This script is called by build.rs during `cargo build`.
    It prefers a globally installed `markmap` / `markmap-cli` and falls back to `npx`.
#>

param(
    [string]$SourceDir = ".doc/markmap",
    [string]$OutputDir = ".doc/markmap/html"
)

$ErrorActionPreference = "Stop"

Write-Host "Building Markmap diagrams..." -ForegroundColor Cyan

if (-not (Test-Path $SourceDir)) {
    Write-Warning "Source directory not found: $SourceDir"
    exit 0
}

New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null

$mmdFiles = Get-ChildItem -Path $SourceDir -Filter "*.mmd" -File

if ($mmdFiles.Count -eq 0) {
    Write-Host "No .mmd files found. Skipping." -ForegroundColor Yellow
    exit 0
}

# Try global markmap first
$markmapCmd = $null
if (Get-Command "markmap" -ErrorAction SilentlyContinue) {
    $markmapCmd = "markmap"
} elseif (Get-Command "markmap-cli" -ErrorAction SilentlyContinue) {
    $markmapCmd = "markmap-cli"
}

foreach ($file in $mmdFiles) {
    $htmlName = [System.IO.Path]::ChangeExtension($file.Name, ".html")
    $htmlPath = Join-Path $OutputDir $htmlName

    Write-Host "  Processing $($file.Name) -> $htmlName" -ForegroundColor Gray

    if ($markmapCmd) {
        & $markmapCmd $file.FullName -o $htmlPath --no-open 2>$null
    } else {
        # Fallback to npx
        npx --yes markmap-cli $file.FullName -o $htmlPath --no-open 2>$null
    }

    if ($LASTEXITCODE -ne 0) {
        Write-Warning "Failed to build $($file.Name)"
    }
}

Write-Host "Markmap build complete. Output: $OutputDir" -ForegroundColor Green