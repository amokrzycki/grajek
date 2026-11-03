param(
    [Parameter(Mandatory)][string]$StageDirectory,
    [Parameter(Mandatory)][string]$ToolsDirectory,
    [Parameter(Mandatory)][string]$Notices,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [Parameter(Mandatory)][string]$Version,
    [string]$Iscc = "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    [switch]$BadYtDlpHash,
    [switch]$BadDenoHash
)
$ErrorActionPreference = 'Stop'
if ((Get-Item $Iscc).VersionInfo.ProductVersion -notmatch '^6\.7\.1\b') {
    throw 'The installer is pinned to Inno Setup 6.7.1. Check the compiler before changing the pin.'
}
$root = (Resolve-Path "$PSScriptRoot\..\..").Path
New-Item -ItemType Directory -Force "$StageDirectory\licenses", $OutputDirectory | Out-Null
$stage = (Resolve-Path $StageDirectory).Path
$output = (Resolve-Path $OutputDirectory).Path
Copy-Item "$root\target\release\grajek.exe", "$root\README.md", "$root\.env.example" $stage
Copy-Item "$ToolsDirectory\components.json", "$ToolsDirectory\components-en.txt", "$ToolsDirectory\components-pl.txt" $stage
Copy-Item "$root\LICENSE" "$stage\licenses\Grajek-LICENSE.txt"
Copy-Item $Notices "$stage\licenses\Grajek-THIRD-PARTY.txt"
Copy-Item "$root\vendor\gpui-pre-linux\LICENSE-APACHE" "$stage\licenses\gpui-LICENSE-APACHE.txt"
Copy-Item "$PSScriptRoot\7zip-LICENSE.txt", "$PSScriptRoot\LGPL-2.1.txt" "$stage\licenses"
$defines = @(Get-Content "$ToolsDirectory\iscc-defines.txt")
if ($BadYtDlpHash) {
    $defines = @($defines | Where-Object { -not $_.StartsWith('/DYtDlpSha256=') }) + ('/DYtDlpSha256=' + ('0' * 64))
}
if ($BadDenoHash) {
    $defines = @($defines | Where-Object { -not $_.StartsWith('/DDenoSha256=') }) + ('/DDenoSha256=' + ('0' * 64))
}
$arguments = @("/DGrajekVersion=$Version", "/DStageDir=$stage", "/O$output") + $defines + @("$PSScriptRoot\grajek.iss")
& $Iscc @arguments
if ($LASTEXITCODE -ne 0) { throw "ISCC failed with exit code $LASTEXITCODE" }
