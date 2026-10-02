# Build the Inno Setup installer for the already-built release binary.
# Usage: build-installer.ps1 -Version <x.y.z>
param([Parameter(Mandatory = $true)][string]$Version)
$ErrorActionPreference = 'Stop'

$iscc = Join-Path "${env:ProgramFiles(x86)}" 'Inno Setup 6\ISCC.exe'
if (-not (Test-Path $iscc)) {
  Write-Host 'Inno Setup 6 not found; installing via Chocolatey'
  choco install innosetup -y --no-progress
  if ($LASTEXITCODE -ne 0) { throw "choco install innosetup failed ($LASTEXITCODE)" }
}
& $iscc /Qp "/DMyAppVersion=$Version" installer\pin.iss
if ($LASTEXITCODE -ne 0) { throw "ISCC.exe failed with exit code $LASTEXITCODE" }
