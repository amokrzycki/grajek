param([Parameter(Mandatory)][string]$Grajek, [Parameter(Mandatory)][string]$ToolsDirectory,
      [Parameter(Mandatory)][string]$BuildLog)
$ErrorActionPreference = 'Stop'
if (Select-String -Path $BuildLog -Pattern 'LNK4098' -Quiet) { throw 'Conflicting CRT link: LNK4098' }
foreach ($binary in @($Grajek, "$ToolsDirectory\deno\unpacked\deno.exe")) {
    $imports = & dumpbin /dependents $binary
    if ($LASTEXITCODE -ne 0) { throw "dumpbin failed for $binary" }
    $imports | Write-Output
    if ($imports -match 'VCRUNTIME140|MSVCP140') { throw "$binary requires an external VC++ runtime" }
}
# Record the PyInstaller dependency tree; DLL loading behavior still needs a clean native VM.
Get-ChildItem "$ToolsDirectory\yt-dlp\unpacked" -Recurse -File |
    Where-Object { $_.Extension -in @('.exe', '.dll', '.pyd') } |
    ForEach-Object {
        Write-Output $_.FullName
        & dumpbin /dependents $_.FullName
        if ($LASTEXITCODE -ne 0) { throw "dumpbin failed for $($_.Name)" }
    }
