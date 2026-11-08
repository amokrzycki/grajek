# 004 — Dowody F0 (2026-11-03)

Status: **F0–F4_IMPLEMENTED — LOCAL_CHECKS_PASS — PENDING_CI — PENDING_NATIVE**.
Nie jest to akceptacja Windows, desktopu, dźwięku ani publicznego wydania.

## Baza i rozbieżności przed edycją

- Odczytano `AGENTS.md` i cały `004-bundled-distribution.md`.
- Początkowa gałąź: `feat/include-libs`; HEAD: `7db3e18`. `git status --short`
  był pusty; `git diff 7db3e18 HEAD` pusty. Utworzono `feat/distribution`.
- Fabryka procesów, argumenty, kontrola Deno ≥2.3, import playlisty,
  `live::prepare`, dotenv, katalog danych, zależności i release workflow
  odpowiadają bazie planu. `title()` ma już komunikat `Could not start yt-dlp.`;
  dopisek `Check PATH.` występuje w `resolve_format()`. Numer wiersza
  `app_data_dir()` przesunięty do 227; semantyka bez zmian.
- Songbird 0.6.1 `input/sources/ytdl.rs::get_stream()` przyjmuje gotowe metadane
  i otwiera HTTP/HLS; nie uruchamia ekstraktora.
- Lokalny `target/release/grajek`: GLIBC maks. **2.44**, NEEDED: libopus.so.0,
  libm.so.6, libxcb.so.1, libxkbcommon.so.0, libxkbcommon-x11.so.0,
  libgcc_s.so.1, libc.so.6, ld-linux-x86-64.so.2. Potwierdza zakaz pakowania
  lokalnego buildu jako zgodnego z Ubuntu 22.04.
- Nagłówek planu nadal mówi „DO AKCEPTACJI”; wiążące jest zatwierdzenie
  użytkownika z 2026-11-03. Decyzje: D1 bez podpisu, D3 mutex w grajek-smtc,
  D4 zachowuje ejs:npm w trybie systemowym. D5 i D7 nadal otwarte.
- `docs/` jest ignorowane przez lokalne `.git/info/exclude`; dowody i checklistę
  należy dodawać przez `git add -f`, bez zmiany reguł repozytorium.
- Wymagania obrazów runnerów z planu nie są tu ponownie potwierdzone przez
  uruchomienie CI. Nie utożsamiamy ich z pomiarami lokalnymi.

## Środowisko i izolacja

Linux x86_64, CachyOS, kernel `7.3.0-4-cachyos`, glibc 2.44,
rustc/cargo 1.99.0. Artefakty w oddzielnych, początkowo pustych katalogach:

```text
/tmp/grajek-f0-8kb9vuKT/yt-linux
/tmp/grajek-f0-8kb9vuKT/yt-windows
/tmp/grajek-f0-8kb9vuKT/deno-linux
/tmp/grajek-f0-8kb9vuKT/deno-windows
```

Podpisy, klucz, logi, rozpakowane narzędzia i cache również poza repo.
Wszystkie skrypty Pythona uruchamiano `python3 -I`. Program PyInstaller
yt-dlp ma własny interpreter, niezależny od systemowego Pythona.
Próby YouTube używają pustego PATH, osobnych HOME/XDG_CONFIG_HOME/
XDG_CACHE_HOME/DENO_DIR. Nie instalowano narzędzi ani nie zmieniano PATH hosta.
Sieć oraz testy wymagające lokalnych socketów uruchomiono poza sandboxem,
który blokował DNS, sockety i część procesów potomnych.

## Piny, autentyczność i rozmiary

Wybrano najnowsze stabilne wydania zwrócone w dniu pomiaru przez API GitHub:

- [yt-dlp 2026.10.26](https://github.com/yt-dlp/yt-dlp/releases/tag/2026.10.26),
  publikacja 2026-10-26; EJS **0.8.0** przypięte w
  [pyproject.toml tego tagu](https://github.com/yt-dlp/yt-dlp/blob/2026.10.26/pyproject.toml).
- [Deno 2.9.8](https://github.com/denoland/deno/releases/tag/v2.9.8),
  publikacja 2026-10-21.

URL-e i pełne SHA-256 w `packaging/components.json` (jedyny manifest pinów).
Rozmiary są w bajtach, unpacked_size = suma ZipInfo.file_size,
bez zaokrąglenia do bloków systemu plików.

| Artefakt | ZIP | Po rozpakowaniu | SHA-256 |
|---|---:|---:|---|
| yt-dlp_linux.zip | 41236547 | 98866816 | `f34053415136e55073dd64ba5326d161ecc23ce0e98c081d8752867b37809450` |
| yt-dlp_win.zip | 23885596 | 41340247 | `fd51b212cf8380c7a52ee08c0d6be7f3c59b2f2c832eaf7b45acfae6eab57419` |
| deno-x86_64-unknown-linux-gnu.zip | 41963064 | 96055992 | `e890975bc2549807bbcc0b584ffafda72cc7c93349dd99c80483dd784f6c4609` |
| deno-x86_64-pc-windows-msvc.zip | 42917472 | 98081568 | `cba84b26e0df5ae7f74fc66b86a95b866872a86292ebd84fa1e959927edd93bf` |

yt-dlp: pobrano `SHA2-256SUMS`, `SHA2-256SUMS.sig` z przypiętego wydania
oraz `public.key` z przypiętego tagu. Osobny GNUPGHOME (0700).

```sh
gpg --batch --homedir "$F0/gnupg" --import "$F0/signatures/public.key"
gpg --batch --no-autostart --homedir "$F0/gnupg" --status-fd 1 \
  --verify "$F0/signatures/SHA2-256SUMS.sig" "$F0/signatures/SHA2-256SUMS"
```

Import w sandboxie dodał klucz, ale zgłosił błąd uruchomienia gpg-agent.
Weryfikacja z `--no-autostart` zakończyła się kodem **0**:

```text
[GNUPG:] GOODSIG 57CF65933B5A7581 Simon Sawicki (yt-dlp signing key) <contact@grub4k.xyz>
[GNUPG:] VALIDSIG AC0CBBE6848D6A873464AF4E57CF65933B5A7581 2026-10-26 1793053481 0 4 0 1 10 00 AC0CBBE6848D6A873464AF4E57CF65933B5A7581
gpg: WARNING: This key is not certified with a trusted signature!
```

Odcisk: **AC0C BBE6 848D 6A87 3464 AF4E 57CF 6593 3B5A 7581**.
Podpis jest poprawny względem pobranego klucza; to nie jest niezależne
poświadczenie tożsamości właściciela. SHA obu ZIP-ów zgadza się z podpisaną listą.
Deno: oba SHA zgadzają się z plikami `.zip.sha256sum` z wydania.
Windowsowy plik ma format PowerShell `Algorithm / Hash / Path`, linuksowy
standardowy format sha256sum. Nie zakładać wspólnego formatu parsera.
Sumy Deno z tego samego wydania nie są niezależną kotwicą zaufania.

## Archiwa i EJS — PASS z rozbieżnością nazwy

Oba ZIP-y yt-dlp zawierają:

```text
_internal/yt_dlp_ejs/yt/solver/core.min.js
_internal/yt_dlp_ejs/yt/solver/lib.min.js
```

Brak symlinków we wszystkich czterech ZIP-ach. Ścieżki sprawdzono przed
rozpakowaniem; przywrócono bity wykonywalności z external_attr.

**Rozbieżność blokująca dalszy projekt:** plik w ZIP-ie Linux to
`yt-dlp_linux`, a nie `yt-dlp`. Windows ma oczekiwane `yt-dlp.exe`.
Bez zmiany nazwy po ekstrakcji lub zmiany resolvera ścieżka z p. 5 i 6.2
planu nie zadziała. **Decyzja właściciela po F0:** zachować nazwę upstream
`tools/yt-dlp/yt-dlp_linux`, bez zmiany nazwy przy rozpakowaniu.

Druga rozbieżność: `yt-dlp -v --version` zwraca wyłącznie `2026.10.26`,
bez listy bibliotek. Kontrola EJS z p. 9 wymaga innego wywołania.
W próbach ekstrakcji `-v` raportuje `yt_dlp_ejs-0.8.0`. Sprawdzono także
zamiennik offline: `--ignore-config --no-plugin-dirs --no-js-runtimes -v
--simulate --load-info-json <plik>`, gdzie plik zawiera:

```json
{"id":"ejs-probe","title":"EJS offline probe","url":"https://example.invalid/probe.mp3","ext":"mp3","extractor":"generic","extractor_key":"Generic","webpage_url":"https://example.invalid/probe.mp3"}
```

Kod **0**, `yt_dlp_ejs-0.8.0` w Optional libraries, bez pobierania mediów.
**Decyzja właściciela:** użyć tej kontroli offline. Wprowadzono ją do
`packaging/check-components.py`; nie uruchamia ekstrakcji YouTube w CI.
Próby pomocnicze z pustym batch-file i pustą playlistą nie nadają się do
kontroli: pierwsza zwraca `You must provide at least one URL.` (kod 2), druga
`yt_dlp.utils.EntryNotInPlaylist: There are no entries` (kod 1).

## ABI Linux — PASS

```sh
objdump -T "$F0/deno-linux/unpacked/deno" > deno-objdump.log
rg -o 'GLIBC_[0-9.]+' deno-objdump.log | sort -Vu
```

Najwyższa wersja Deno: **GLIBC_2.27**. `deno --version` (kod 0):

```text
deno 2.9.8 (stable, release, x86_64-unknown-linux-gnu)
v8 15.0.245.2-rusty
typescript 6.0.3
```

yt-dlp_linux: główny ELF maks. 2.14; sprawdzono też wszystkie ELF-y `_internal`,
cały zestaw wymaga maks. **GLIBC_2.17**. Piny nie podnoszą proponowanej granicy
Grajka z Ubuntu 22.04 (2.35). Pomiar symboli nie zastępuje uruchomienia na 22.04.

## BBB z jawnym Deno, bez remote-components — PASS

`F0` oznacza katalog pomiaru. Przed próbą utworzono puste katalogi
empty-path, probe-home, probe-cache i deno-cache. Te same zmienne obowiązują
dla kolejnej próby playlisty.

```sh
export PATH="$F0/empty-path" HOME="$F0/probe-home"
export XDG_CONFIG_HOME="$F0/probe-home" XDG_CACHE_HOME="$F0/probe-cache"
export DENO_DIR="$F0/deno-cache"
"$F0/yt-linux/unpacked/yt-dlp_linux" \
  --ignore-config --no-plugin-dirs --socket-timeout 15 --retries 0 \
  --extractor-retries 0 -v --no-playlist \
  --js-runtimes "deno:$F0/deno-linux/unpacked/deno" -j \
  'https://www.youtube.com/watch?v=aqz-KE-bpKQ'
```

Kod **0**, poprawny JSON: id `aqz-KE-bpKQ`, tytuł
`Big Buck Bunny 60fps 4K - Official Blender Foundation Short Film`, 61 formatów.
Istotne linie stderr:

```text
[debug] JS runtimes: deno-2.9.8
[debug] Plugin directories: none (disabled)
[youtube] [jsc:deno] Solving JS challenges using deno
[debug] [youtube] [jsc:deno] Using challenge solver lib script v0.8.0 (source: python package, variant: minified)
[debug] [youtube] [jsc:deno] Using challenge solver core script v0.8.0 (source: python package, variant: minified)
```

Deno uruchomiono po ścieżce bezwzględnej z `--no-remote --no-npm --cached-only`.
Nie przekazano `--remote-components`. Ostrzeżenia: `Unknown codec iamf.001.001.Opus`
oraz brak ffmpeg (próba `-j`, bez pobierania audio). To nie jest test dźwięku ani
streamingu Discorda.

## Flat-playlist bez Deno — PASS

Pusty PATH jak wyżej; dodatkowo jawne `--no-js-runtimes`, żeby wykluczyć
autodetekcję runtime. Playlista używana przez istniejący test integracyjny.

```sh
"$F0/yt-linux/unpacked/yt-dlp_linux" \
  --ignore-config --no-plugin-dirs --socket-timeout 15 --retries 0 \
  --extractor-retries 0 -v --no-js-runtimes --flat-playlist \
  --dump-single-json --playlist-end 3 --lazy-playlist --skip-download \
  --yes-playlist 'https://www.youtube.com/playlist?list=PLBB231211A4F62143'
```

Kod **0**, `_type=playlist`, id `PLBB231211A4F62143`, 3 pozycje.
Log: `[debug] JS runtimes: none (disabled)`. Runtime nie jest potrzebny
do tego importu; brak podstaw do dodawania go do argumentów playlisty.

## Kontrole Rust

Uruchomione na niezmienionym kodzie bazowym, po pomiarach F0:

| Polecenie | Wynik |
|---|---|
| `cargo fmt --all -- --check` | PASS, kod 0 |
| `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | PASS, kod 0 |
| `cargo test --locked --workspace --all-features` w sandboxie | FAIL, kod 101; 292 passed, 8 failed, 6 ignored |
| To samo `cargo test` poza sandboxem | PASS, kod 0; Grajek 300 passed, 0 failed, 6 ignored; grajek-smtc i doc-tests: 0 testów |

Pełna lista błędów pierwszej próby (nie ukrywamy niepowodzenia):

```text
media::range::tests::failed_probes_keep_safe_diagnostics_and_allow_songbird_fallback
media::range::tests::later_range_failure_is_signalled_after_reading_or_seeking
media::range::tests::ignored_misaligned_truncated_or_changed_ranges_are_rejected
media::range::tests::ranges_support_both_directions_eof_and_overflow_safe_offsets
  Error: Operation not permitted (os error 1)
native::linux::tests::isolated_bus_exposes_interfaces_commands_properties_and_releases_name
tray::linux::tests::icons_follow_watch_updates_without_reregistering_or_refreshing_unchanged_visuals
  Error: address error: Invalid address. See https://dbus.freedesktop.org/doc/dbus-specification.html#addresses
playlist::tests::cancellation_kills_the_owned_subprocess
  Error: child exited before ready
playlist::tests::subprocess_exit_timeout_and_both_output_bounds_are_enforced
  thread panicked at src/playlist/tests.rs:117:5:
  assertion `left == right` failed
    left: Err(Unavailable)
   right: Err(Timeout)
test result: FAILED. 292 passed; 8 failed; 6 ignored
error: test failed, to rerun pass `-p grajek --bin grajek`
```

Powtórzenie poza sandboxem usuwa wszystkie osiem błędów bez zmian kodu.
Nie uruchamiano sześciu testów ignored. Żaden z wyników nie potwierdza Windows.

## PENDING_NATIVE i dalsze kroki

- **F0-WIN-CRT:** build `+crt-static`, dumpbin Grajka, Deno i `_internal`, brak LNK4098.
- **F0-WIN-INNO:** natywna próba download + Hash + extractarchive, zły skrót,
  offline/ponów/anuluj, usuwanie tools, zachowanie danych i notka 7-Zip.
- **F5-WINDOWS / F5-LINUX / F5-DISCORD:** wszystkie scenariusze p. 12.2,
  z działającym odtwarzaniem i odsłuchem na drugim kliencie.

Procedury oraz niezaznaczona checklista: `004-native-checklist.md`.
Obie rozbieżności F0 zostały rozstrzygnięte przez właściciela.
**D7:** wersja 0.6.0, instalator PL i EN. **D5:** zachować portable ZIP/tar.gz
przez wydanie przejściowe obok Setup i AppImage.
Nie zmieniono vendor/gpui-pre-linux ani zakazu unsafe w głównej crate.
Nie wykonano push, tagowania, wydania, PR ani uruchomienia release workflow.

## F1 — resolver

Dodano wybór kompletnego zestawu System/Managed po dotenv, marker wersji,
typowane błędy brakujących składników, wspólną fabrykę poleceń i argumenty
z jawną ścieżką Deno w trybie Managed. System zachowuje ejs:npm (D4).
Oba tryby wyłączają pluginy. Brak/nieczytelny marker wersji nie powoduje
przełączenia uszkodzonego zestawu zarządzanego na PATH.

Każda ekstrakcja zachowuje własny snapshot ścieżek i argumentów. Inicjalizacja
przyjmuje katalog od main, więc samodzielny seek-spike nadal importuje media
bez zależności od całej warstwy store. Pełne kontrole wersji pozostają przed Play.
Zmiana ścieżki yt-dlp w playlistach nie dodaje runtime JS.

Testy RED→GREEN: marker wybiera Managed również przy brakujących plikach;
system odrzuca pustą wersję yt-dlp i wyjście innego programu zamiast Deno.
Testy obejmują override, uszkodzony marker, ścieżki Linux/Windows, argumenty,
wersje z zainstalowanego markera (starsze piny nadal dozwolone) i typ błędu.

- `cargo fmt --all -- --check`: PASS.
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`: PASS.
- `cargo test --locked --workspace --all-features` poza sandboxem: PASS,
  **305 passed, 0 failed, 6 ignored**; grajek-smtc/doc-tests po 0.
- Windows CI/native, cargo run na prawdziwym desktopie i Discord: niewykonane.

## F3 — pobieranie Linux i AppImage

Dodano pobieranie strumieniowe HTTPS z limitami rozmiaru, SHA-256, bezpiecznym
ZIP (odrzucanie traversal, pomijanie symlinków, bity wykonywalności), kontrolą
wersji i markerem publikowanym na końcu. Plik blokady zapobiega konfliktom
dwóch instancji. Restart odzyskuje tools.old po przerwaniu między rename.
Resolver zmienia komplet dopiero po powodzeniu, respektując GRAJEK_TOOLS=system.

UI wymaga kliknięcia przed pobraniem; pokazuje źródła, licencje, wersje, rozmiar,
postęp, Retry i Cancel. Można wrócić do kolejki podczas pobierania. Not now
nie wraca przy każdym snapshotcie; nowa próba Play/importu ponownie otwiera
prompt przez typowany sygnał kontrolera. Settings pokazuje wybrany tryb.

Testy obejmują: poprawny komplet, zły SHA, przerwaną sieć, Cancel w połowie,
traversal, nadmierny rozmiar, symlinki, blokadę, odzyskiwanie tools.old,
kontrolę startową bez procesów, odroczenie promptu i zachowanie odtwarzania
podczas nieudanego importu. Nowe zależności Linux-only: sha2 0.10.9, zip 8.6.0
z samym backendem DEFLATE (bez zopfli). Wersja aplikacji: 0.6.0 (D7).

Skrypt check-components: trzy testy stdlib PASS; pełne rzeczywiste pobranie
Linux do `/tmp/grajek-check-components-zVtNz5uV` PASS: SHA, oba rozmiary,
wersje, EJS offline 0.8.0, maks. GLIBC yt-dlp 2.17 i Deno 2.27.

Build AppImage przypina również appimagetool 1.9.1 i type2-runtime 20251108
w tym samym components.json. Skrypt odrzuca ELF > GLIBC 2.35, sprawdza RPATH
oraz brak narzędzi w wypakowanym AppDir. Pełny check-appdir bez --elf-only
uruchamia dodatkowo ldd w czystym ubuntu:22.04. Ta kontrola jest osobnym
krokiem, żeby builder kontenerowy nie potrzebował socketu Dockera hosta.

Kontrole Rust dla F3: `cargo fmt --all -- --check` PASS;
`cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` PASS;
`cargo test --locked --workspace --all-features` poza sandboxem PASS:
**317 passed, 0 failed, 6 ignored**, grajek-smtc/doc-tests po 0.
Walidacja rzeczywistego pakietu Ubuntu 22.04 zakończyła się powodzeniem;
wynik końcowej paczki po poprawce recenzji jest w sekcji F4 poniżej.

## F2 — Setup Windows (implementacja, PENDING_NATIVE)

Dodano per-user Setup PL/EN z niezmiennym AppId, bez UAC i bez zmian PATH.
Piny i teksty zgody generuje check-components; pliki narzędzi pobiera sam
Setup z Hash i extractarchive, marker instaluje na końcu. AppMutex wskazuje
`Local\org.grajek.desktop`; guard w grajek-smtc utrzymuje go także w tray.
FFI jest wyłącznie w tej crate, główna crate nadal ma unsafe_code=forbid.
Statyczny CRT jest w osobnym `.cargo/config.toml` dla targetu MSVC.

Przygotowano skrypty budowania, dumpbin oraz natywnego smoke testu Setup:
dwa złe skróty, poprawna instalacja, wersje narzędzi, deinstalacja i zachowany
state.json. Nie uruchamiano ich na Linux przez Wine ani inną symulację.
Notki 7-Zip/LGPL pobrano z oficjalnych stron; zgodność z faktycznym is7z.dll
w Inno 6.7.1 nadal wymaga natywnej kontroli F0-WIN-INNO.

- fmt, clippy z wymaganymi flagami: PASS.
- `cargo test --locked --workspace --all-features` poza sandboxem: PASS,
  **317 passed, 0 failed, 6 ignored** (test mutexu Windows tu nie działa).
- `cargo check --locked -p grajek-smtc --all-targets --all-features --target x86_64-pc-windows-msvc`:
  PASS, w tym typowanie testu czasu życia mutexu. To **cross-check** bez
  linkowania i uruchamiania, nie dowód CRT, Setup ani zachowania Windows.
- ISCC, dumpbin, instalacja/deinstalacja, testy mutexu i F5: **PENDING_NATIVE**.

## F4 — workflow, dokumentacja i końcowa walidacja

Wspólny `package.yml` służy CI dla zmian packaging/Cargo/.cargo/workflow oraz
release. Buduje natywnie Linux/Windows, sprawdza przypięte narzędzia, generuje
licencje, buduje Setup/AppImage i przejściowe ZIP/tar.gz. Przygotowano natywne
kontrole dumpbin, ISCC i instalatory z dwoma błędnymi SHA. Release rozwiązuje
istniejący tag do SHA, weryfikuje komplet SHA256SUMS i tworzy wyłącznie draft.
**Nie uruchomiono workflow_dispatch**, nie utworzono tagu, release, PR ani push.
Próba tych workflow na GitHubie pozostaje **PENDING_CI** zgodnie z zakresem pracy.

README opisuje oba sposoby instalacji, tryby narzędzi, zgody, aktualizację,
naprawę, odinstalowanie i zachowane dane. packaging/README zawiera procedurę
pinów/GPG, budowania oraz generowania notek. D5/D7 są zamknięte decyzjami
właściciela; żadne decyzje projektowe nie czekają już na odpowiedź.

### Wyniki końcowe

| Kontrola | Wynik |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | PASS |
| `cargo test --locked --workspace --all-features` poza sandboxem | **319 passed, 0 failed, 6 ignored**; SMTC/doc-tests po 0 |
| `python3 -I packaging/test-components.py` | **4 passed** |
| actionlint 1.7.12, wszystkie 3 workflow, `-shellcheck=` | PASS; opcjonalny ShellCheck nie był uruchamiany |
| `bash -n packaging/linux/build-appimage.sh` | PASS; skrypt również wykonano przy rzeczywistym pakowaniu |
| PowerShell 7.6.9 Linux, Parser.ParseFile dla 3 skryptów .ps1 | PASS; wyłącznie AST/składnia, bez wykonania komend Windows |
| cargo-about 0.9.3, graf Linux i Windows, --locked --all-features | PASS; pełne notki wygenerowane |
| SMTC `cargo check --locked -p grajek-smtc --all-targets --all-features --target x86_64-pc-windows-msvc` | PASS; bez linkowania/uruchamiania Windows |
| Build release w kontenerze Ubuntu 22.04 | PASS; pierwszy pełny build 9m04s, końcowy rebuild po poprawkach 29.35s |
| AppImage, wypakowanie, 5 ELF-ów, GLIBC ≤2.35, RPATH | PASS |
| `ldd` w osobnym czystym ubuntu:22.04, bez sieci | PASS względem listy wykluczeń; tylko hostowy libxcb.so.1 nieobecny |
| Lokalny SHA256SUMS dla AppImage, `sha256sum --check` | PASS; nie jest to sprawdzenie nieutworzonych assetów Windows |

Końcowy AppImage (bez yt-dlp/Deno):

```text
/tmp/grajek-linux-2204-8c5d6scm/out/Grajek-0.6.0-x86_64.AppImage
27728376 bytes
SHA-256 0eca960c1ea2a9995c70a0249e1d1c1338408de84df3032e8dc844592fe050a5
```

Źródła w builderze były kopią roboczych źródeł 0.6.0 po poprawce recenzji,
bez .git, .env i target z hosta. Builder nie miał socketu Dockera hosta.
Artefakt jest lokalny; nie został wysłany do żadnego wydania.
Logi: `/tmp/grajek-linux-final-build.log`, `/tmp/grajek-appimage-final-package.log`,
`/tmp/grajek-appdir-final-validation.log`, `/tmp/grajek-f4-test-final.log`,
`/tmp/grajek-f4-clippy-final.log`. Pomiary narzędzi F0 nadal są w osobnych
katalogach `/tmp/grajek-f0-8kb9vuKT`.

Listę bibliotek hosta porównano z
https://raw.githubusercontent.com/AppImage/pkg2appimage/master/excludelist
w dniu pomiaru (SHA-256 pliku:
`50db0f894f34b169c47a5cbc0c17dbab61e9edebca5bc8269a1e6ac1bf4bdad9`).
Cztery dołączane biblioteki nie są na upstreamowej liście.
Vulkan i libwayland-egl są jawnymi wyjątkami hostowymi planu. Usunięto zbędne
wyjątki Xau/Xdmcp/bsd/md; każdy nierozwiązany import spoza pozostałej listy
przerywa kontrolę, podobnie jak błąd dowolnego wywołania ldd.

### Recenzja i poprawki regresji

Jedna świeża recenzja całej gałęzi znalazła **1 P2**: blokada ekstraktorów
działała tylko wewnątrz procesu. Publikacja mogła usunąć `_internal` używane
przez drugą instancję. Naprawiono w jednej turze: współdzielona blokada pliku
chroni managed subprocessy; publikacja i odzyskiwanie wymagają wyłącznej.
Pod lokalną blokadą procesową publikacja utrzymuje też blokadę zasobów i
instalatora aż do przełączenia resolvera. Wersje markera są odczytywane pod
blokadą, więc zmiana przez drugą instancję nie wywołuje fałszywej naprawy.
Oczekiwanie można anulować, nie blokując wątku UI.

Testy RED→GREEN obejmują własny proces i oddzielny `flock --shared`.
Oczekiwane błędy RED, usunięte przez poprawki:

```text
Published while an extractor was still running
Replaced another process's active tools
test_ejs_version_must_match_the_library_token_exactly: AssertionError: True is not false
```

W ostatnim przypadku kontrola EJS akceptowała 0.8.01 jako prefiks 0.8.0;
teraz porównuje cały token biblioteki z właściwej linii debug. Sprawdzono też
końcowy parser z prawdziwym przypiętym yt-dlp offline: PASS 0.8.0.
Recenzent nie zgłosił innych konkretnych błędów. Poprawki zweryfikowano testami,
bez zlecania drugiej recenzji. Minory odroczone: brak.

### Decyzje techniczne wykonawcy i ich granice

1. Katalog narzędzi przekazuje main do resolvera: korzysta z istniejącego store,
   bez uzależniania samodzielnego seek-spike od całej aplikacji. Błędny katalog
   uniemożliwiłby odnalezienie managed tools; main zachowuje ścieżki z planu.
2. Odzyskiwanie tools.old i bezpieczne blokady File/stdlib uzupełniają dwa rename.
   Chronią poprzedni komplet i dwie instancje; koszt to odroczenie podmiany
   podczas aktywnego ekstraktora. Cancel przerywa oczekiwanie.
3. zip 8.6.0 korzysta tylko z backendu DEFLATE, bez zbędnego zopfli. Zmiana
   metody kompresji przez upstream wymaga zmiany backendu i ponownych testów;
   dotychczasowe przypięte ZIP-y są obsługiwane, bez cichego pomijania błędów.
   Listing czterech rzeczywistych ZIP-ów potwierdził metody 0 (stored) i 8 (DEFLATE).
4. Build i kontrola kontenerowa są osobnymi krokami. Automatyczny review odrzucił
   montowanie socketu Docker (zbyt szerokie uprawnienia hosta). Bezpieczniejszy
   wariant wykonał build bez socketu, a osobny ldd uruchomiono z hosta. Żadna
   bramka testowa nie została przez to pominięta.

### Pozostałe bramki

- **PENDING_NATIVE F0-WIN-CRT / F0-WIN-INNO:** native build/link, dumpbin,
  działanie mutexu, ISCC, dobre i złe SHA, cleanup, offline/ponów/anuluj,
  zgodność notki is7z oraz zachowanie danych.
- **PENDING_NATIVE F5:** Windows 10/11, Ubuntu GNOME Wayland/X11, KDE
  Wayland/X11/XWayland, tray, MPRIS/SMTC i rzeczywisty odsłuch Discorda.
- **PENDING_CI:** uruchomienie przygotowanych workflow, pełny komplet assetów
  i release SHA256SUMS; nie wykonano z powodu zakazu push/tag/release w tym zadaniu.
- Przed publikacją zachować dostępność źródeł i materiału do ponownego linkowania
  części LGPL statycznego type2-runtime; pochodzenie tekstów i źródeł zapisano
  w packaging/linux/runtime-licenses/SOURCES.md. Sam smoke test czystej instalacji
  nie dowodzi zachowania starego zestawu po nieudanej aktualizacji Windows.
- Guard AppMutex pojawia się dopiero w 0.6.0. Bazowy portable 0.5.0 nie może
  zostać wykryty przez nieistniejący w nim mutex: trzeba go zamknąć ręcznie.
  Test blokady N-1 wymaga poprzedniego buildu z guardem; migracja z 0.5.0 jest
  osobnym scenariuszem. Nie zastąpiono D3 enumeracją procesów ani kodem Pascal.

Nie zmieniono vendor/gpui-pre-linux, src/window.rs, build.rs, windows.rc ani
Makefile. Główna crate nadal ma `unsafe_code = "forbid"`. Wszystkie scenariusze
ręczne pozostają niezaznaczone w osobnej checkliście.

## F0-WIN-CRT — zależności narzędzi Windows (2026-11-03)

Commit `87e26c9`. Windows 11 x64, MSVC 19.51.36231 (BuildTools 18),
SDK 10.0.26100.0, dumpbin 14.51.36257.0. Pierwszy pomiar zależności narzędzi
nie zależy od Rusta. Grajek był początkowo w `target/release` z Rust 1.98.1;
domyślny toolchain podniesiono do **1.99.0** (`rustc 1.99.0 b940084d7`,
`cargo 1.99.0 5f94df478`, LLVM 23.1.1) i przebudowano
`cargo build --locked --release`: PASS w 3m17s, binarka z 16:10.

Artefakty pobrano `curl.exe` do pustego katalogu poza repo,
`C:\Users\Adrian\AppData\Local\Temp\grajek-f0-tools`, przed rozpakowaniem:

```text
cba84b26e0df5ae7f74fc66b86a95b866872a86292ebd84fa1e959927edd93bf  deno.zip
fd51b212cf8380c7a52ee08c0d6be7f3c59b2f2c832eaf7b45acfae6eab57419  yt-dlp_win.zip
```

Oba SHA-256 zgadzają się z `packaging/components.json`. Rozmiary po
rozpakowaniu: yt-dlp **41340247**, Deno **98081568** — zgodne z manifestem.
`deno --version` → `2.9.8 (stable, release, x86_64-pc-windows-msvc)`;
`yt-dlp.exe --version` → `2026.10.26`.

### Deno

`dumpbin /dependents deno.exe`: **brak VCRUNTIME140, VCRUNTIME140_1,
MSVCP140, CONCRT140**. Importy to wyłącznie systemowe: bcryptprimitives,
kernel32, api-ms-win-core-synch-l1-2-0, user32, ntdll, advapi32, ws2_32,
crypt32, userenv, shell32, bcrypt. Delay-load: dbghelp, gdi32, opengl32,
setupapi, oleaut32, combase, rstrtmgr, iphlpapi, psapi, winmm. Deno jest
samowystarczalne; nie potrzebuje VC++ redist.

### yt-dlp i `_internal`

`yt-dlp.exe` to bootloader PyInstaller: ADVAPI32, KERNEL32, msvcrt, USER32.
Cała reszta jest w `_internal` (144 pliki, w tym 121 `.exe/.dll/.pyd`).
Sprawdzono `dumpbin /dependents` na **wszystkich 121** plikach binarnych.

- VCRUNTIME140 i VCRUNTIME140_1 są **dołączone** w `_internal`
  (14.51.36247.0, 178616 i 50112 bajtów). `MSVCP140`, `MSVCP140_1`
  i `CONCRT140` nie występują nigdzie — ani jako import, ani jako plik.
- Pozostałe importy CRT to `api-ms-win-crt-*` (Universal CRT, składnik
  Windows 10/11), a nie osobny redistributable.
- Pozostałe niesystemowe importy: `python314.dll`, `libcrypto-3.dll`,
  `libssl-3.dll`, `libffi-8.dll`, `sqlite3.dll`,
  `libcurl-impersonate-ae864b926b0a86bfa3b18a664407a1ab.dll`.
  Wszystkie leżą w `_internal`; każdy import rozwiązywały pliki zestawu,
  poza systemowym `normaliz.dll` i api-setami.
- `_wmi.pyd` importuje `VCRUNTIME140_1.dll` i ole32 (delay-load), oba
  dostępne.

Wniosek: zestaw narzędzi nie wymaga instalacji VC++ redist na Windows 10/11;
własne DLL ładuje z `_internal`.

### Grajek

`target/release/grajek.exe` (przebudowany Rustem 1.99.0, 16:10): importy
wyłącznie systemowe — kernel32, advapi32, user32, shell32,
gdi32, comctl32, ws2_32, ntdll, icuuc, uiautomationcore, oleaut32, combase,
ole32, dcomp, dwmapi, dwrite, winmm, imm32, dxgi, d3d11, bcrypt,
bcryptprimitives, gdiplus, userenv i api-sety. **Brak VCRUNTIME140
i MSVCP140.** Logu LNK4098 z tego builda nie zachowano; brak trafienia
wynika z braku całej grupy importów CRT.

Logi i listingi: `dependents.log`, `SHA256SUMS.txt`, `listing.txt`
w katalogu pomiaru. **Nie wykonano** punktu czystej VM bez redist:
wnioski powyżej to analiza importów, nie dowód uruchomienia.

## Setup Windows zbudowany natywnie (2026-11-03)

Build z commita `87e26c9` plus niezacommitowaną poprawką bramki ISCC opisaną
niżej. Windows 11 x64, Rust 1.99.0, MSVC 19.51.36231, SDK 10.0.26100.0.

Narzędzia buildu:

- Inno Setup **6.7.1** zainstalowany per-user z
  `github.com/jrsoftware/issrc/releases/download/is-6_7_1/innosetup-6.7.1.exe`
  (`SHA-256 4d11e8050b6185e0d49bd9e8cc661a7a59f44959a621d31d11033124c4e8a7b0`,
  10619024 B), cicho, `/CURRENTUSER`. Rejestr:
  `DisplayVersion 6.7.1`, katalog
  `%LOCALAPPDATA%\Programs\Inno Setup 6`. Wersja 6.7.3/6.7.4 z winget
  i jrsoftware nie została użyta — pin to 6.7.1.
- Python 3.12.10 (winget, user scope) i `cargo-about 0.9.3 --features cli`.

**Błąd bramki naprawiony.** `ISCC.exe` 6.7.1 nie ma zasobu wersji:
`VersionInfo.ProductVersion` = `0.0.0.0`, a `ISCC /?` nie wypisuje numeru.
Dotychczasowa kontrola w `build-installer.ps1` odrzucałaby więc każdy
prawdziwy 6.7.1. Zamiast tego skrypt czyta `DisplayVersion` wpisu
odinstalowania (`Inno Setup 6_is1`) z HKLM, HKLM\WOW6432Node i HKCU, nadal
wymagając `^6\.7\.1`. Dodano fallback ścieżki ISCC do instalacji per-user.
Pin wersji bez zmian.

Wyniki:

| Krok | Wynik |
|---|---|
| `cargo build --locked --release` w czystym `CARGO_TARGET_DIR` | PASS, 3m32s |
| `Select-String -Pattern LNK4098` w logu builda | brak trafień |
| `check-crt.ps1` (Grajek, deno, 121 plików `_internal`) | PASS |
| `check-components.py --platform windows` | PASS: hash, oba rozmiary, `--version`, EJS 0.8.0 offline |
| `python -I packaging/test-components.py` | 4 PASS |
| `cargo about generate` (Windows target) | PASS, notka 806579 B |
| ISCC | Successful compile, 8,297 s |

Artefakt:

```text
dist\Grajek-Setup-0.6.0.exe
13772660 bytes
SHA-256 b59dc91bceb03e6d4e5f18a3b1f3667754c52b8a633b6d6e01a7e182740a0363
```

Instalator nie jest podpisany i nie był uruchamiany. Pobiera oba ZIP-y
z URL-i manifestu, weryfikuje `Hash`, rozpakowuje do `{app}\tools`
i publikuje `components.json` na końcu. **PENDING_NATIVE:** instalacja na
czystej VM, fixture'y z `-BadYtDlpHash`/`-BadDenoHash`, offline/ponów/anuluj,
cicha instalacja i deinstalacja, zachowanie `state.json`, zgodność notki
`is7z.dll` z faktycznie dołączanym plikiem.

Katalog `dist\` nie jest w `.gitignore`; artefakt pozostaje nieśledzony.

### Fałszywy marker i brak monitu o składnikach — poprawione

Zgłoszenie właściciela: instalacja bez internetu, pobrania pominięte, a Grajek
mimo to raportuje komplet „Grajek-managed” z wersjami; w trybie system PATH,
gdy narzędzi nie ma, nie pojawia się żaden monit.

Przyczyny (odczyt kodu, nie domysł):

1. `grajek.iss` kopiował `tools\components.json` jako zwykły wpis `[Files]`,
   bez warunku. Marker wersji istniał więc nawet wtedy, gdy oba ZIP-y nie
   zostały rozpakowane.
2. `Tools::select` traktuje sam marker jako Managed, a `description()` brała
   wersje z markera bez sprawdzenia plików — Settings pokazywało wersje dla
   pustego `tools`.
3. Monit o brakujących składnikach na Windows był tylko tekstem w liście
   historii (`state.notice`, 2 linie, mała czcionka). Ekran startowy składników
   (`with_components`) był wyłącznie linuksowy, więc nic nie „wyskakiwało”.

Poprawki:

- `grajek.iss`: marker ma `Check: ToolsInstalled`; funkcja w `[Code]` wymaga
  `{app}\tools\yt-dlp\yt-dlp.exe` i `{app}\tools\deno\deno.exe`. Bez obu
  plików marker nie powstaje.
- `src/media/components.rs`: wspólne `missing()` dla trybu Managed i System;
  `description()` zgłasza „needs repair, run Grajek Setup again” zamiast
  wersji, gdy plików brak.
- `src/main.rs` + `src/app.rs`: Windows pokazuje pełnoekranowy monit
  „Playback components needed” przed odtwarzaczem, z przyciskiem OK.
  Komunikat nadal trafia też do `state.notice`, więc jest widoczny po
  zamknięciu.

Weryfikacja lokalna (Windows 11, Rust 1.99.0):

| Kontrola | Wynik |
|---|---|
| `cargo test --locked -p grajek --bin grajek components` | 9 PASS, w tym nowy `a_marker_without_binaries_asks_for_repair_instead_of_claiming_versions` |
| `cargo test --locked --workspace --all-features` | 306 + 3 PASS, 6 ignored |
| `cargo fmt --all -- --check`, `cargo clippy ... -D warnings` | PASS |
| UI test `missing_components_are_announced_before_the_player` | PASS (monit obecny, OK zamyka) |
| Ręcznie: katalog z `grajek.exe` + samym `tools\components.json` | monit widoczny natychmiast, zrzut `%TEMP%\grajek-broken-check\window.png` |
| ISCC po zmianie `.iss` | Successful compile |

Nowy Setup: `dist\Grajek-Setup-0.6.0.exe`, 13761855 B,
SHA-256 `2474c96b2ccc710816d052f4768872032bf694a23e85e581e8652a64f78ef257`.
**PENDING_NATIVE:** zachowanie `Check` na prawdziwej instalacji offline oraz
to, że po pominiętych pobraniach marker nie istnieje. Nie uruchamiano Setup na
VM; powyżej jest kompilacja i testy aplikacji, nie dowód instalatora.
