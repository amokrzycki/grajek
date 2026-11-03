# 004 — Checklista natywna (PENDING_NATIVE)

Stan początkowy 2026-11-03: **żaden punkt poniżej nie jest zaliczony**.
Pomiary Linux F0 są w `004-evidence.md`. Cross-build i runner CI nie zamykają
tej checklisty. Po każdym teście zapisać datę, commit, OS, wersje, komendy,
wynik i lokalizację logów/zrzutów. Dowody akceptacji wydania docelowo trafiają
do `docs/releases/v0.6.0.md` (D7: 0.6.0, instalator PL/EN).

## F0-WIN-CRT — natywny build i zależności

Osobna maszyna developerska Windows x64 z MSVC/SDK i Rust 1.99.0; następnie
czysta VM do uruchomienia. Nie stosować Wine ani cross-kompilacji jako dowodu.

- [ ] Zapisać commit oraz `rustc -Vv`, `cargo -V`, wersję Visual Studio/SDK.
- [ ] W Developer PowerShell, z czystym katalogiem wyjściowym buildu:

  ```powershell
  $env:CARGO_TARGET_DIR = Join-Path $env:TEMP ('grajek-f0-crt-' + [guid]::NewGuid())
  $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS = '-C target-feature=+crt-static'
  cargo build --locked --release --target x86_64-pc-windows-msvc 2>&1 |
    Tee-Object -FilePath "$env:CARGO_TARGET_DIR-build.log"
  if ($LASTEXITCODE -ne 0) { throw 'Native build failed' }
  $exe = Join-Path $env:CARGO_TARGET_DIR 'x86_64-pc-windows-msvc\release\grajek.exe'
  dumpbin /dependents $exe | Tee-Object -FilePath "$env:CARGO_TARGET_DIR-dependents.log"
  Select-String -Path "$env:CARGO_TARGET_DIR-build.log" -Pattern LNK4098
  Select-String -Path "$env:CARGO_TARGET_DIR-dependents.log" -Pattern 'VCRUNTIME140|MSVCP140'
  ```

  Żadne trafienie LNK4098/VCRUNTIME140/MSVCP140 nie jest akceptowalne.
  Zmienne ustawić w osobnej sesji PowerShell; po teście zamknąć sesję.
- [ ] Pobrać Windowsowe ZIP-y z URL-i manifestu do **oddzielnych pustych
  katalogów poza repo**, sprawdzić `Get-FileHash -Algorithm SHA256` względem
  manifestu, dopiero wtedy rozpakować. Zapisać listingi i sumy.
- [ ] `dumpbin /dependents deno.exe`; rekurencyjnie `dumpbin /dependents`
  dla `yt-dlp.exe` i wszystkich `.exe`, `.dll`, `.pyd` w `_internal`.
  Zapisać dla każdego pliku importy i obecność potrzebnych DLL obok niego.
- [ ] `deno.exe --version` i `yt-dlp.exe --version` odpowiadają pinom.
- [ ] Przenieść zestaw na czystą VM bez VC++ redist; uruchomić Grajka i narzędzia.
  Jeśli Deno potrzebuje systemowego CRT albo statyczny build zawodzi:
  **STOP — decyzja właściciela o planie B**, bez instalowania redist po cichu.

## F0-WIN-INNO — minimalny test download + extractarchive

Na czystej VM Windows, standardowe konto; budowanie fixture może odbyć się
na osobnym Windows z Inno Setup **6.7.1**. Zapisać wersję `ISCC.exe`.
Fixture ma osobny AppId i osobny katalog `Grajek-F0-Fixture`, nigdy katalog danych.

- [ ] Utworzyć poza repo pusty katalog fixture, w nim `probe.iss`:

  ```iss
  [Setup]
  AppId={{7B78BF73-4F0F-4C79-A98B-41FA989398D0}
  AppName=Grajek F0 Fixture
  AppVersion=0.0.0
  PrivilegesRequired=lowest
  DefaultDirName={localappdata}\Programs\Grajek-F0-Fixture
  ArchitecturesAllowed=x64compatible
  ArchitecturesInstallIn64BitMode=x64compatible
  ArchiveExtraction=full
  OutputBaseFilename=Grajek-F0-Fixture
  CloseApplications=no

  [Files]
  Source: "{#YtDlpUrl}"; DestDir: "{tmp}"; DestName: "yt-dlp_win.zip"; ExternalSize: {#YtDlpZipSize}; Hash: "{#YtDlpSha256}"; Flags: external download ignoreversion deleteafterinstall
  Source: "{tmp}\yt-dlp_win.zip"; DestDir: "{app}\tools\yt-dlp"; ExternalSize: {#YtDlpUnpackedSize}; Flags: external extractarchive recursesubdirs createallsubdirs ignoreversion
  Source: "{#DenoUrl}"; DestDir: "{tmp}"; DestName: "deno.zip"; ExternalSize: {#DenoZipSize}; Hash: "{#DenoSha256}"; Flags: external download ignoreversion deleteafterinstall
  Source: "{tmp}\deno.zip"; DestDir: "{app}\tools\deno"; ExternalSize: {#DenoUnpackedSize}; Flags: external extractarchive recursesubdirs createallsubdirs ignoreversion
  Source: "components.json"; DestDir: "{app}\tools"; Flags: ignoreversion

  [InstallDelete]
  Type: filesandordirs; Name: "{app}\tools"

  [UninstallDelete]
  Type: filesandordirs; Name: "{app}\tools"
  ```

- [ ] Utworzyć lokalny `components.json` jako marker wersji
  `{"yt-dlp":"2026.10.26","yt-dlp-ejs":"0.8.0","deno":"2.9.8"}`.
- [ ] Wywołać ISCC z `/DYtDlpUrl`, `/DYtDlpZipSize`, `/DYtDlpSha256`,
  `/DYtDlpUnpackedSize`, `/DDenoUrl`, `/DDenoZipSize`, `/DDenoSha256`,
  `/DDenoUnpackedSize` z manifestu `packaging/components.json`, potem `probe.iss`.
  Zachować pełny command line i log kompilatora. Jeśli flagi są niezgodne,
  zapisać błąd i wrócić do decyzji o `CreateDownloadPage` + `ExtractArchive`.
- [ ] Uruchomić fixture na czystej VM z `/LOG=<absolutny-log>`: pobranie,
  rozpakowanie obu narzędzi i marker na końcu; brak UAC. Sprawdzić `--version`.
- [ ] Snapshot → fixture z SHA yt-dlp równym 64 zerom. Instalacja ma zawieść;
  brak `tools/components.json` oraz brak częściowego katalogu tools.
- [ ] Powtórzyć z prawidłowym yt-dlp i SHA Deno równym 64 zerom. Ustalić,
  czy wcześniejszy yt-dlp został już rozpakowany; nie uznawać samego braku
  markera za dowód braku częściowych plików.
- [ ] Snapshot → offline. Ponów po włączeniu sieci; osobno Anuluj.
  Sprawdzić brak częściowych plików. Zachować logi i zrzuty.
- [ ] Cicha instalacja `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /LOG=...`,
  następnie deinstalacja tymi samymi flagami: katalog tools usunięty.
- [ ] Przed testami utworzyć kontrolny `%LOCALAPPDATA%\Grajek\state.json`
  z rozpoznawalną treścią. Instalacja/deinstalacja fixture go nie zmienia.
- [ ] Zanotować licencję i notkę 7-Zip dla `is7z.dll` dołączanej przez użyte
  Inno; ustalić plik z tekstem, który trafi do finalnego instalatora.

## F5 — środowiska do odhaczenia osobno

Każdy scenariusz poniżej ma osobny wynik dla każdego wymaganego środowiska.
Brak GPU/Discorda oznacza PENDING, nie PASS.

- [ ] Windows 11 x64, czysta VM, standardowe konto, bez Python/yt-dlp/Deno/redist.
- [ ] Windows 10 22H2 x64, czysta VM, standardowe konto.
- [ ] Ubuntu 22.04, GNOME Wayland, bez systemowych narzędzi i libfuse2.
- [ ] Ubuntu 22.04, GNOME X11, te same warunki.
- [ ] CachyOS/Arch, KDE Plasma 6 Wayland/XWayland.
- [ ] CachyOS/Arch, KDE Plasma 6 X11.
- [ ] Opcjonalnie Fedora, GNOME Wayland.

### Windows (dla obu wersji systemu)

- [ ] Zapisać `where.exe python yt-dlp deno`, brak klucza
  `HKLM\SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64` oraz snapshot VM.
- [ ] SmartScreen/Defender, instalacja bez UAC, treść strony składników PL/EN,
  wersje i rozmiary, postęp. `reg query HKCU\Environment /v Path` i
  `reg query "HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment" /v Path`
  przed i po: PATH bez zmian.
- [ ] Offline → czytelny błąd; Ponów po przywróceniu sieci; Anuluj sprząta tools.
- [ ] Konfiguracja bota/kanału, publiczny film, **odsłuch na drugim kliencie Discord**,
  brak migających konsol, Settings pokazuje Grajek-managed.
- [ ] Pause/Resume, seek w obie strony, Volume, Next, Repeat, live bez duration.
- [ ] Import playlisty ≤100 i odtworzenie jej pozycji, kolejka/historia.
- [ ] Tray Hide/Show, Show po minimalizacji, Close-to-tray, Quit, sterowanie audio.
- [ ] SMTC: tytuł, klawisze Play/Pause/Next, timeline, brak timeline dla live.
- [ ] Aktualizacja N-1 ukrytego w tray: AppMutex blokuje Setup; po Quit aktualizacja
  działa, nowe wersje tools, zachowana kolejka, historia, ustawienia i token.
  Dla blokady mutexu N-1 musi już zawierać guard (0.6.0+). Bazowe 0.5.0 go nie
  tworzy; migrację z takiego portable sprawdzić osobno po ręcznym Quit.
- [ ] Usunięte deno.exe → komunikat naprawy; ponowny Setup naprawia narzędzia.
- [ ] Deinstalacja podczas działania aplikacji blokowana mutexem. Po Quit usuwa
  app/tools/skróty; zachowuje dane, .env użytkownika i Credential Manager.
  Ponowna instalacja przywraca kolejkę.
- [ ] Defender: skan katalogu instalacji, zapisane wykrycia/false positives.

### Linux (osobno każda sesja Wayland i X11)

- [ ] `command -v yt-dlp deno` puste; AppImage startuje bez libfuse2,
  pokazuje okno składników. Zapisać glibc, GPU i sterownik Vulkan.
- [ ] Not now → Play ponownie otwiera okno; Settings ma Download.
- [ ] Download: widoczny postęp, Cancel w połowie, po restarcie tools.partial
  sprzątnięte, stary stan zachowany; ponowny Download instaluje komplet.
- [ ] Offline → błąd/Ponów. Build testowy ze złym SHA → błąd sumy,
  brak nowego kompletu i zachowany poprzedni.
- [ ] **Odsłuch Discorda na drugim kliencie**: Play, seek, live, import i Next.
- [ ] KDE: `python3 -I scripts/check-tray-linux.py` na Wayland i X11/XWayland;
  ręczne Hide/Show/Quit oraz Show z minimalizacji. Nie używać zatrzymanej
  fixture jako dowodu audio.
- [ ] MPRIS: `playerctl -p grajek status`, `metadata`, `play-pause`, `next`;
  widgety systemowe KDE i GNOME.
- [ ] Open Developer Portal uruchamia przeglądarkę hosta bez błędów bibliotek.
- [ ] Arch z systemowymi yt-dlp/deno/yt-dlp-ejs: brak okna, Settings system PATH.
  Osobno wymuszenie `GRAJEK_TOOLS=system` przy istniejącym katalogu managed.
- [ ] Nowszy pin AppImage: Update; Later zachowuje działające stare narzędzia;
  Update podmienia komplet. Przerwanie podczas podmiany nie traci starego zestawu.
- [ ] Bez FUSE: zapisać błąd runtime, potem sprawdzić `--appimage-extract-and-run`.
- [ ] Zanotować ikonę KDE Wayland bez integracji desktopowej (znane ograniczenie).

## Reguła publikacji

- [ ] Przed publiczną dystrybucją zapewnić dostęp do odpowiadających źródeł
  i materiału do ponownego linkowania części LGPL statycznego type2-runtime;
  sprawdzić zapis źródeł/notek w packaging/linux/runtime-licenses/SOURCES.md.

Wszystkie powyższe wyniki pozostają **PENDING_NATIVE** do wykonania na tych
systemach. F5 nie jest częścią lokalnej automatycznej walidacji. W tej pracy
nie tworzymy tagów, release, push ani PR; ewentualny przyszły release ma
pozostać szkicem do czasu akceptacji natywnej.
