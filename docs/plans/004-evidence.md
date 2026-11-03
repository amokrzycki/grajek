# 004 — Dowody F0 (2026-11-03)

Status: **F0_LINUX_MEASURED — DESIGN_DECISION_REQUIRED — PENDING_NATIVE**.
F1–F4 nie rozpoczęto. Nie jest to akceptacja Windows ani wydania.

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
planu nie zadziała. Nie wybrano samodzielnie wariantu; pytanie do właściciela.

Druga rozbieżność: `yt-dlp -v --version` zwraca wyłącznie `2026.10.26`,
bez listy bibliotek. Kontrola EJS z p. 9 wymaga innego wywołania.
W próbach ekstrakcji `-v` raportuje `yt_dlp_ejs-0.8.0`. Sprawdzono także
zamiennik offline: `--ignore-config --no-plugin-dirs --no-js-runtimes -v
--simulate --load-info-json <plik>`, gdzie plik zawiera:

```json
{"id":"ejs-probe","title":"EJS offline probe","url":"https://example.invalid/probe.mp3","ext":"mp3","extractor":"generic","extractor_key":"Generic","webpage_url":"https://example.invalid/probe.mp3"}
```

Kod **0**, `yt_dlp_ejs-0.8.0` w Optional libraries, bez pobierania mediów.
To propozycja zastąpienia kontroli z p. 9, nie wprowadzona zmiana skryptu CI.
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
Najpierw rozstrzygnąć obie rozbieżności F0; dopiero po zgodzie przejść do F1.
Nie zmieniono vendor/gpui-pre-linux ani zakazu unsafe w głównej crate.
Nie wykonano push, tagowania, wydania, PR ani uruchomienia release workflow.
