; Build with ISCC 6.7.1. All upstream pin values come from check-components.py.
#ifndef GrajekVersion
  #error GrajekVersion is required
#endif
#ifndef StageDir
  #error StageDir is required
#endif

[Setup]
AppId={{713B0B2A-8BF1-4EAD-AF33-B222E4625B3D}
AppName=Grajek
AppVersion={#GrajekVersion}
AppVerName=Grajek
AppPublisher=Adrian Mokrzycki
DefaultDirName={localappdata}\Programs\Grajek
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.15063
ArchiveExtraction=full
AppMutex=Local\org.grajek.desktop
CloseApplications=no
UninstallDisplayIcon={app}\grajek.exe
SetupIconFile=..\..\assets\brand\grajek-windows.ico
LicenseFile={#StageDir}\licenses\Grajek-LICENSE.txt
OutputBaseFilename=Grajek-Setup-{#GrajekVersion}
Compression=lzma2
SolidCompression=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"; InfoBeforeFile: "{#StageDir}\components-en.txt"
Name: "polish"; MessagesFile: "compiler:Languages\Polish.isl"; InfoBeforeFile: "{#StageDir}\components-pl.txt"

[Tasks]
Name: "desktopicon"; Description: "{cm:DesktopShortcut}"; Flags: unchecked

[Files]
Source: "{#StageDir}\grajek.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\licenses\*"; DestDir: "{app}\licenses"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#YtDlpUrl}"; DestDir: "{tmp}"; DestName: "yt-dlp_win.zip"; ExternalSize: {#YtDlpZipSize}; Hash: "{#YtDlpSha256}"; Flags: external download ignoreversion deleteafterinstall
Source: "{tmp}\yt-dlp_win.zip"; DestDir: "{app}\tools\yt-dlp"; ExternalSize: {#YtDlpUnpackedSize}; Flags: external extractarchive recursesubdirs createallsubdirs ignoreversion
Source: "{#DenoUrl}"; DestDir: "{tmp}"; DestName: "deno.zip"; ExternalSize: {#DenoZipSize}; Hash: "{#DenoSha256}"; Flags: external download ignoreversion deleteafterinstall
Source: "{tmp}\deno.zip"; DestDir: "{app}\tools\deno"; ExternalSize: {#DenoUnpackedSize}; Flags: external extractarchive recursesubdirs createallsubdirs ignoreversion
Source: "{#StageDir}\components.json"; DestDir: "{app}\tools"; Flags: ignoreversion; Check: ToolsInstalled

[InstallDelete]
Type: filesandordirs; Name: "{app}\tools"
Type: files; Name: "{app}\.env.example"
Type: files; Name: "{autoprograms}\Grajek.lnk"
Type: files; Name: "{autoprograms}\Grajek\Grajek.lnk"
Type: files; Name: "{autoprograms}\Grajek\Uninstall Grajek.lnk"
Type: files; Name: "{autoprograms}\Grajek\Odinstaluj Grajka.lnk"
Type: dirifempty; Name: "{autoprograms}\Grajek"

[UninstallDelete]
Type: filesandordirs; Name: "{app}\tools"

[Icons]
Name: "{autoprograms}\Grajek\Grajek"; Filename: "{app}\grajek.exe"; WorkingDir: "{app}"
Name: "{autoprograms}\Grajek\{cm:UninstallShortcut}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\Grajek"; Filename: "{app}\grajek.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\grajek.exe"; Description: "{cm:LaunchProgram,Grajek}"; Flags: postinstall nowait skipifsilent

[CustomMessages]
english.DesktopShortcut=Create a desktop shortcut
polish.DesktopShortcut=Utwórz skrót na pulpicie
english.UninstallShortcut=Uninstall Grajek
polish.UninstallShortcut=Odinstaluj Grajka

[Messages]
english.InfoBeforeLabel=Playback components
polish.InfoBeforeLabel=Składniki odtwarzania
english.SetupAppRunningError=Grajek is running, possibly hidden in the tray. Choose Quit from the tray menu, then click OK.
polish.SetupAppRunningError=Grajek działa, być może ukryty w zasobniku. Wybierz Zakończ z menu zasobnika, a następnie kliknij OK.
english.UninstallAppRunningError=Grajek is running, possibly hidden in the tray. Choose Quit from the tray menu, then click OK.
polish.UninstallAppRunningError=Grajek działa, być może ukryty w zasobniku. Wybierz Zakończ z menu zasobnika, a następnie kliknij OK.
english.UninstalledAll=Grajek and its downloaded components were removed. Your settings, queue and history remain in %%LOCALAPPDATA%%\Grajek; your bot token remains in Windows Credential Manager.
polish.UninstalledAll=Grajek został usunięty razem z pobranymi składnikami. Twoje ustawienia, kolejka i historia pozostały w %%LOCALAPPDATA%%\Grajek, a token bota w Menedżerze poświadczeń Windows.

[Code]
// The version marker must never exist without the tools it describes, otherwise
// Grajek reports installed components for an empty tools directory.
function ToolsInstalled: Boolean;
begin
  Result := FileExists(ExpandConstant('{app}\tools\yt-dlp\yt-dlp.exe')) and
    FileExists(ExpandConstant('{app}\tools\deno\deno.exe'));
end;
