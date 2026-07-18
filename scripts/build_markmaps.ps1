# scripts/build_markmaps.ps1
# Rebuilds all Mermaid diagrams from .mmd sources
# Uses npx @mermaid-js/mermaid-cli
# Usage: .\scripts\build_markmaps.ps1

$ErrorActionPreference = "Continue"
$diagramDir = ".grok/diagrams"

if (-not (Test-Path $diagramDir)) {
    Write-Host "No .grok/diagrams directory found." -ForegroundColor Yellow
    exit 0
}

$mmdFiles = Get-ChildItem -Path $diagramDir -Filter "*.mmd" | Sort-Object Name

if ($mmdFiles.Count -eq 0) {
    Write-Host "No .mmd files found in $diagramDir." -ForegroundColor Yellow
    exit 0
}

$count = $mmdFiles.Count
Write-Host "Found $count diagram file(s) in $diagramDir..." -ForegroundColor Cyan
Write-Host "Using npx @mermaid-js/mermaid-cli" -ForegroundColor Green
Write-Host ""

$success = 0
$failed = 0

foreach ($mmd in $mmdFiles) {
    $svg = [System.IO.Path]::ChangeExtension($mmd.FullName, "svg")
    Write-Host "-> $($mmd.Name)" -NoNewline -ForegroundColor White

    # Clean command - no extra puppeteer flags
    & cmd /c npx --yes @mermaid-js/mermaid-cli -i $mmd.FullName -o $svg --quiet | Out-Null

    if ($LASTEXITCODE -eq 0) {
        Write-Host "  OK" -ForegroundColor Green
        $success = $success + 1
    } else {
        Write-Host "  FAIL" -ForegroundColor Red
        Write-Host "   Try manually: npx --yes @mermaid-js/mermaid-cli -i $($mmd.FullName) -o $svg --quiet" -ForegroundColor DarkGray
        $failed = $failed + 1
    }
}

Write-Host ""
Write-Host ("Done: {0} succeeded, {1} failed" -f $success, $failed) -ForegroundColor Cyan

if ($failed -gt 0) {
    Write-Host ""
    Write-Host "Tip: If you see Puppeteer/Chrome errors, try cleaning temp folders:" -ForegroundColor Yellow
    Write-Host "Remove-Item -Path `"$env:TEMP\puppeteer_dev_chrome_profile*`" -Recurse -Force" -ForegroundColor DarkGray
}
