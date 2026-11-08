param([Parameter(Mandatory)][string]$Setup, [Parameter(Mandatory)][string]$BadYtDlpSetup,
      [Parameter(Mandatory)][string]$BadDenoSetup)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path "$PSScriptRoot\..\..").Path
$pins = Get-Content "$root\packaging\components.json" -Raw | ConvertFrom-Json
$fixture = Join-Path ([IO.Path]::GetTempPath()) ('grajek-installer-test-' + [guid]::NewGuid())
New-Item -ItemType Directory $fixture | Out-Null
$install = Join-Path $fixture 'Grajek'
$data = Join-Path $env:LOCALAPPDATA 'Grajek\state.json'
if (Test-Path $data) { throw 'Use a clean Windows test account: state.json already exists.' }
if (Test-Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{713B0B2A-8BF1-4EAD-AF33-B222E4625B3D}_is1') {
    throw 'Use a clean Windows test account: Grajek is already installed.'
}
New-Item -ItemType Directory -Force (Split-Path $data) | Out-Null
$sentinel = '{"distribution-test":"preserve-user-data"}'
[IO.File]::WriteAllText($data, $sentinel)
function Invoke-Setup([string]$Executable, [string]$Name) {
    $log = Join-Path $fixture "$Name.log"
    $process = Start-Process -FilePath (Resolve-Path $Executable).Path -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/DIR=`"$install`"", "/LOG=`"$log`"") -PassThru -Wait
    return $process.ExitCode
}
foreach ($bad in @($BadYtDlpSetup, $BadDenoSetup)) {
    $name = [IO.Path]::GetFileName((Split-Path $bad))
    $exitCode = Invoke-Setup $bad $name
    if ($exitCode -eq 0) { throw 'An invalid hash was accepted' }
    if (Test-Path "$install\tools") { throw 'Invalid download left a partial tools directory' }
}
if ((Invoke-Setup $Setup 'install') -ne 0) { throw 'Silent installation failed' }
if (Test-Path "$install\.env.example") { throw 'Installation includes the developer environment example' }
$programs = [Environment]::GetFolderPath('Programs')
if (Test-Path "$programs\Grajek.lnk") { throw 'Installation left a flat Start Menu shortcut' }
$shell = New-Object -ComObject WScript.Shell
if ($shell.CreateShortcut("$programs\Grajek\Grajek.lnk").TargetPath -ne "$install\grajek.exe") { throw 'Start Menu application shortcut is missing or targets the wrong executable' }
$uninstallLinks = @(Get-ChildItem "$programs\Grajek\*.lnk" | Where-Object { $shell.CreateShortcut($_.FullName).TargetPath -like "$install\unins*.exe" })
if ($uninstallLinks.Count -ne 1) { throw 'Start Menu must contain one uninstaller shortcut' }
$marker = Get-Content "$install\tools\components.json" -Raw | ConvertFrom-Json
if ($marker.'yt-dlp' -ne $pins.'yt-dlp'.version -or $marker.deno -ne $pins.deno.version) { throw 'Installed marker versions differ from pins' }
$ytVersion = & "$install\tools\yt-dlp\yt-dlp.exe" --version
if ($LASTEXITCODE -ne 0 -or $ytVersion.Trim() -ne $pins.'yt-dlp'.version) { throw 'Installed yt-dlp failed its version check' }
$denoVersion = & "$install\tools\deno\deno.exe" --version
if ($LASTEXITCODE -ne 0 -or $denoVersion[0] -notmatch ('^deno ' + [regex]::Escape($pins.deno.version) + ' ')) { throw 'Installed Deno failed its version check' }
if ([IO.File]::ReadAllText($data) -ne $sentinel) { throw 'Installation changed user data' }
$uninstaller = Get-ChildItem "$install\unins*.exe" | Select-Object -First 1
$process = Start-Process -FilePath $uninstaller.FullName -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/LOG=`"$fixture\uninstall.log`"") -Wait -PassThru
if ($process.ExitCode -ne 0 -or (Test-Path $install)) { throw 'Uninstall did not remove the installation and tools' }
if (Test-Path "$programs\Grajek") { throw 'Uninstall left the Start Menu folder' }
if ([IO.File]::ReadAllText($data) -ne $sentinel) { throw 'Uninstall removed or changed user data' }
Remove-Item $data
Write-Output "Installer smoke tests passed; logs: $fixture. Native GUI/audio acceptance is still pending."
