# Build release binaries for Nano Snipper

Write-Host "Building release binaries..." -ForegroundColor Cyan

cargo build --release --workspace 2>&1

if ($LASTEXITCODE -eq 0) {
    Write-Host "`nRelease build successful!" -ForegroundColor Green
    Write-Host "Binaries:"
    Write-Host "  nanosnipper.exe:  target\release\nanosnipper.exe"
    Write-Host "  snipui.exe: target\release\snipui.exe"

    $nanosnipperSize = (Get-Item "target\release\nanosnipper.exe" -ErrorAction SilentlyContinue).Length / 1MB
    $snipuiSize = (Get-Item "target\release\snipui.exe" -ErrorAction SilentlyContinue).Length / 1MB
    Write-Host "`nSizes:"
    Write-Host "  nanosnipper.exe:  $([math]::Round($nanosnipperSize, 2)) MB"
    Write-Host "  snipui.exe: $([math]::Round($snipuiSize, 2)) MB"
} else {
    Write-Host "`nRelease build failed." -ForegroundColor Red
}
