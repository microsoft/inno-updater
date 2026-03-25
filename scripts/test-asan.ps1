#!/usr/bin/env pwsh
# Run tests with AddressSanitizer to detect memory bugs
#
# ASAN catches:
# - Use-after-free (like the to_u16s().as_ptr() bug)
# - Buffer overflows
# - Stack/heap buffer overflows
# - Memory leaks

param(
    [switch]$Install,
    [switch]$Verbose
)

$ErrorActionPreference = "Stop"

# ASAN only works on x86_64, not i686
$target = "x86_64-pc-windows-msvc"

function Install-AsanComponents {
    # Find VS Installer
    $vsWhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    if (-not (Test-Path $vsWhere)) {
        Write-Host "Error: Visual Studio Installer not found." -ForegroundColor Red
        exit 1
    }

    # Use -products * to include Build Tools, not just full VS IDE
    $vsPath = & $vsWhere -products * -latest -property installationPath
    if (-not $vsPath) {
        Write-Host "Error: No Visual Studio installation found." -ForegroundColor Red
        exit 1
    }
    
    $vsInstaller = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vs_installer.exe"
    
    if (-not (Test-Path $vsInstaller)) {
        Write-Host "Error: vs_installer.exe not found." -ForegroundColor Red
        exit 1
    }

    Write-Host "Installing C++ AddressSanitizer component..." -ForegroundColor Cyan
    Write-Host "VS Installation: $vsPath" -ForegroundColor Gray
    Write-Host "This will open a Visual Studio Installer window." -ForegroundColor Gray
    
    # Component ID for C++ AddressSanitizer
    $asanComponent = "Microsoft.VisualStudio.Component.VC.ASAN"
    
    # Run installer in modify mode to add the component
    # Use a single string for ArgumentList to handle paths with spaces correctly
    $installerArgs = "modify --installPath `"$vsPath`" --add $asanComponent --passive"
    
    Start-Process -FilePath $vsInstaller -ArgumentList $installerArgs -Wait
    
    if ($LASTEXITCODE -ne 0) {
        Write-Host "VS Installer exited with code $LASTEXITCODE" -ForegroundColor Yellow
    }
}

if ($Install) {
    Write-Host "Installing Rust nightly toolchain..." -ForegroundColor Cyan
    rustup install nightly
    rustup +nightly target add $target
    rustup +nightly component add rust-src
    
    # Check if ASAN is already installed
    $vsPath = & "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe" -products * -latest -property installationPath 2>$null
    $asanLib = Get-ChildItem -Path "$vsPath\VC\Tools\MSVC\*\lib\x64\clang_rt.asan*" -ErrorAction SilentlyContinue
    
    if (-not $asanLib) {
        Write-Host "`nASAN runtime not found. Installing via VS Installer..." -ForegroundColor Yellow
        Install-AsanComponents
    } else {
        Write-Host "ASAN runtime already installed." -ForegroundColor Green
    }
    
    Write-Host "`nInstallation complete!" -ForegroundColor Green
    exit 0
}

# Check prerequisites
$nightlyInstalled = rustup show | Select-String "nightly"
if (-not $nightlyInstalled) {
    Write-Host "Error: Nightly toolchain not installed. Run: .\scripts\test-asan.ps1 -Install" -ForegroundColor Red
    exit 1
}

# Check for ASAN library
$vsPath = & "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe" -products * -latest -property installationPath 2>$null
if ($vsPath) {
    $asanLib = Get-ChildItem -Path "$vsPath\VC\Tools\MSVC\*\lib\x64\clang_rt.asan*" -ErrorAction SilentlyContinue
    if (-not $asanLib) {
        Write-Host "Error: C++ AddressSanitizer not found. Run: .\scripts\test-asan.ps1 -Install" -ForegroundColor Red
        exit 1
    }
    
    # Add ASAN library path to linker search path
    $asanLibDir = Split-Path $asanLib[0].FullName -Parent
    $env:LIB = "$asanLibDir;$env:LIB"
    Write-Host "ASAN lib path: $asanLibDir" -ForegroundColor Gray
    
    # Add ASAN DLL path to PATH for runtime
    $asanDll = Get-ChildItem -Path "$vsPath\VC\Tools\MSVC\*\bin\Hostx64\x64\clang_rt.asan*.dll" -ErrorAction SilentlyContinue
    if ($asanDll) {
        $asanDllDir = Split-Path $asanDll[0].FullName -Parent
        $env:PATH = "$asanDllDir;$env:PATH"
        Write-Host "ASAN DLL path: $asanDllDir" -ForegroundColor Gray
    }
}

Write-Host "Running tests with AddressSanitizer..." -ForegroundColor Cyan
Write-Host "Target: $target" -ForegroundColor Gray

$env:RUSTFLAGS = "-Zsanitizer=address"

# Reduce ASAN quarantine size to catch use-after-free more aggressively
# quarantine_size_mb=0 makes freed memory immediately reusable
$env:ASAN_OPTIONS = "quarantine_size_mb=0"

$cargoArgs = @(
    "+nightly",
    "test",
    "--target", $target,
    "-Zbuild-std"
)

if ($Verbose) {
    $cargoArgs += "--verbose"
}

Write-Host "cargo $($cargoArgs -join ' ')" -ForegroundColor DarkGray

& cargo @cargoArgs

if ($LASTEXITCODE -eq 0) {
    Write-Host "`nASAN tests passed!" -ForegroundColor Green
} else {
    Write-Host "`nASAN detected issues or build failed!" -ForegroundColor Red
    exit $LASTEXITCODE
}
