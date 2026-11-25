# Build dist\Triceratops-<version>-windows-x64.zip.
# Needs Rust (MSVC toolchain), Visual Studio Build Tools (C++ workload) and CMake.
#
#   pwsh scripts/package-windows.ps1
$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root

$Version = $env:VERSION
if (-not $Version) {
    $Version = (Select-String -Path Cargo.toml -Pattern '^version = "(.*)"' | Select-Object -First 1).Matches[0].Groups[1].Value
}

cargo build --release --locked --bin triceratops
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$Stage = Join-Path $Root 'target\windows\Triceratops'
Remove-Item -Recurse -Force $Stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $Stage, (Join-Path $Root 'dist') | Out-Null
Copy-Item target\release\triceratops.exe $Stage
Copy-Item README.md $Stage

$Zip = Join-Path $Root "dist\Triceratops-$Version-windows-x64.zip"
Remove-Item -Force $Zip -ErrorAction SilentlyContinue
Compress-Archive -Path $Stage -DestinationPath $Zip
Write-Host "built $Zip"
