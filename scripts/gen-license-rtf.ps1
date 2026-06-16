# Generate wix/License.rtf from the repository LICENSE file for the MSI
# license agreement dialog. Uses simple RTF (WordPad-compatible) to avoid
# the blank-license bug with complex Word-generated RTF.
param(
    [string]$LicensePath = (Join-Path $PSScriptRoot "..\LICENSE"),
    [string]$OutputPath = (Join-Path $PSScriptRoot "..\wix\License.rtf")
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path $LicensePath)) {
    throw "LICENSE file not found: $LicensePath"
}

function Escape-Rtf([string]$Text) {
    return $Text.Replace("\", "\\").Replace("{", "\{").Replace("}", "\}")
}

$lines = Get-Content -Path $LicensePath -Encoding UTF8
$body = ($lines | ForEach-Object { "$(Escape-Rtf $_)\par" }) -join "`r`n"

$rtf = @"
{\rtf1\ansi\deff0
{\fonttbl{\f0\fmodern Courier New;}}
\f0\fs18
$body
}
"@

$outDir = Split-Path $OutputPath -Parent
if (-not (Test-Path $outDir)) {
    New-Item -ItemType Directory -Force -Path $outDir | Out-Null
}

[System.IO.File]::WriteAllText($OutputPath, $rtf, [System.Text.UTF8Encoding]::new($false))
Write-Host "Wrote $($lines.Count) license lines to $OutputPath"