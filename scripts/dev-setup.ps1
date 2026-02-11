# Dev setup script for Nano Snipper
# Prerequisites: Rust toolchain (rustup), Visual Studio Build Tools

Write-Host "Checking prerequisites..." -ForegroundColor Cyan

# Check Rust
if (!(Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Host "ERROR: cargo not found. Install Rust: https://rustup.rs" -ForegroundColor Red
    exit 1
}

Write-Host "Rust: $(rustc --version)" -ForegroundColor Green

# Check Windows SDK (for windows crate)
$sdkPath = "C:\Program Files (x86)\Windows Kits\10"
if (Test-Path $sdkPath) {
    Write-Host "Windows SDK found" -ForegroundColor Green
} else {
    Write-Host "WARNING: Windows SDK not found at expected path" -ForegroundColor Yellow
}

# Build workspace
Write-Host "`nBuilding workspace..." -ForegroundColor Cyan
cargo build --workspace 2>&1

if ($LASTEXITCODE -eq 0) {
    Write-Host "`nBuild successful!" -ForegroundColor Green
} else {
    Write-Host "`nBuild failed. Check errors above." -ForegroundColor Red
}
