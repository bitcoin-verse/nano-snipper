# build-installer.ps1 — Build Nano Snipper and create installer
# Requires: Rust toolchain, Inno Setup 6 (iscc.exe in PATH or default install location)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$ProjectRoot = Split-Path -Parent $PSScriptRoot

Write-Host "=== Building Nano Snipper (release) ===" -ForegroundColor Cyan
Push-Location $ProjectRoot
try {
    cargo build --workspace --release
    if ($LASTEXITCODE -ne 0) {
        Write-Error "Cargo build failed"
        exit 1
    }
} finally {
    Pop-Location
}

# Verify binaries exist
$Binaries = @(
    "$ProjectRoot\target\release\nanosnipper.exe",
    "$ProjectRoot\target\release\snipui.exe"
)
foreach ($bin in $Binaries) {
    if (-not (Test-Path $bin)) {
        Write-Error "Binary not found: $bin"
        exit 1
    }
    $size = (Get-Item $bin).Length / 1MB
    Write-Host ("  Found: $bin ({0:N1} MB)" -f $size)
}

# Find iscc.exe
$IsccPaths = @(
    "iscc.exe",  # In PATH
    "G:\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
)

$Iscc = $null
foreach ($path in $IsccPaths) {
    if (Get-Command $path -ErrorAction SilentlyContinue) {
        $Iscc = $path
        break
    }
    if (Test-Path $path) {
        $Iscc = $path
        break
    }
}

if (-not $Iscc) {
    Write-Error "Inno Setup compiler (iscc.exe) not found. Install Inno Setup 6 from https://jrsoftware.org/isinfo.php"
    exit 1
}

Write-Host "`n=== Building installer ===" -ForegroundColor Cyan
Write-Host "  Using: $Iscc"

$IssFile = "$ProjectRoot\installer\nanosnipper.iss"
& $Iscc $IssFile
if ($LASTEXITCODE -ne 0) {
    Write-Error "Inno Setup compilation failed"
    exit 1
}

$Output = "$ProjectRoot\installer\Output\NanoSnipperSetup.exe"
if (Test-Path $Output) {
    $size = (Get-Item $Output).Length / 1MB
    Write-Host "`n=== Done ===" -ForegroundColor Green
    Write-Host ("  Installer: $Output ({0:N1} MB)" -f $size)
} else {
    Write-Error "Installer not created"
    exit 1
}
