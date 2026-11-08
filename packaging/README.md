# Distribution packaging

Target: Grajek 0.6.0, Windows x64 Setup (PL/EN) and Linux x86_64 AppImage.
Portable ZIP/tar.gz remain for this transition release (D5). No package contains
yt-dlp or Deno: Setup or the Linux app downloads pinned upstream ZIPs after
disclosure. `components.json` is the single source of URLs, hashes and sizes,
including the AppImage build tools. Runtime tools are never added to PATH.

Implementation and measured results, and the native acceptance checklist, are kept
outside this repository. The presence of these scripts, cross-checks or a green
runner does not establish clean-Windows, desktop, tray or live Discord behavior.
F5 stays manual.

## Updating playback pins

1. Create separate empty download directories outside this repository, one per
   upstream artifact. Run all Python commands with `-I`.
2. Choose explicit stable release tags, never `latest` or `continuous` URLs.
3. For yt-dlp, download both ZIPs, `SHA2-256SUMS` and `SHA2-256SUMS.sig` from
   the release. Obtain `public.key` from the corresponding upstream source tag.
   Use an isolated GNUPGHOME and check the fingerprint before accepting it:

   ```text
   AC0C BBE6 848D 6A87 3464 AF4E 57CF 6593 3B5A 7581
   ```

   ```sh
   gpg --homedir "$GNUPGHOME" --import public.key
   gpg --homedir "$GNUPGHOME" --fingerprint
   gpg --homedir "$GNUPGHOME" --no-autostart --verify SHA2-256SUMS.sig SHA2-256SUMS
   ```

   Stop on a different fingerprint or invalid signature. The recorded key comes
   from the pinned upstream tag; it has not been certified by an independent
   trust chain. Copy only hashes matching that signed checksum list.
4. Read `yt-dlp-ejs`'s exact version in that tag's pyproject.toml. Confirm both
   ZIPs contain `_internal/yt_dlp_ejs/yt/solver/{core,lib}.min.js`.
   Preserve the Linux filename **yt-dlp_linux**, beside `_internal`.
5. For Deno, compare hashes with each release's `.zip.sha256sum`. The Windows
   file can use PowerShell's `Hash : ...` format, unlike Linux's sha256sum line.
   These files share the download origin and are not an independent trust anchor.
   The Deno `mirror` entry points at Deno's own download server (`dl.deno.land`,
   the one its install script uses). That server repacks the ZIP: equal size and
   contents, different SHA-256. Update `mirror.url`, `mirror.sha256` and
   `mirror.size` together and compare them with that server's own
   `.zip.sha256sum`.
6. Update `version`, `ejs`, `sha256`, `size` (ZIP bytes) and `unpacked_size`
   (sum of ZIP entry sizes) in the manifest. Verify with independent local
   SHA-256 and ZIP listings before committing; the repository hash is the
   installer's trust anchor.
7. Run the checker **on each native platform**, using a new empty directory:

   ```sh
   python3 -I packaging/test-components.py
   python3 -I packaging/check-components.py --platform linux --directory /tmp/grajek-new-pin
   ```

   On Windows, use `--platform windows` and add
   `--iscc-defines <absolute-output-path>` for installer defines. It also writes
   the installed version marker and PL/EN disclosure files populated from pins.
   A Windows checker cannot be run by executing PE files through Wine on Linux.
8. The checker confirms hashes, both sizes, versions, EJS via a local
   `--simulate --load-info-json` fixture, and ELF GLIBC requirements on Linux.
   `-v --version` alone does **not** list EJS. For a changed Deno pin, compare
   the measured GLIBC floor with 2.35 and update requirements as necessary.
9. Repeat the manual playback scenarios on Windows and Linux. YouTube extraction
   in CI is informational; neither a network failure nor a successful `-j` is
   evidence of Discord audio.

Changing runtime pins does not update already installed tools silently. Windows
gets the new pair through a new Setup. Linux prompts Update and keeps old tools
on Later. `GRAJEK_TOOLS=system` remains an explicit diagnostic override; managed
failures never mix the two pairs or fall back to PATH automatically.

## Rust license notices

Use cargo-about **0.9.3**, with its explicit `cli` feature:

```sh
cargo install cargo-about --version 0.9.3 --features cli --locked
cargo about generate --locked --all-features --target x86_64-unknown-linux-gnu \
  -c packaging/about.toml packaging/about.hbs -o /tmp/Grajek-THIRD-PARTY.txt
```

Use `x86_64-pc-windows-msvc` for the Windows package. License graph inspection
works without executing Windows code. First use may need network access for
Cargo metadata even with `--target`; offline mode requires all those sources
already cached. The generated text includes license texts and crate attribution.
Build/dev dependencies are excluded from the distributed runtime notice.

Each package also includes Grajek's MIT license and the vendored GPUI Apache
license. AppImage includes Debian copyright files for its four shared libraries
and the upstream type2-runtime notice. Setup includes 7-Zip/LGPL notices; confirm
their match with the installed Inno 6.7.1 `is7z.dll` during native acceptance.
Update source/license records together with upstream build-tool pins.

## Linux AppImage

Build on **Ubuntu 22.04**, not on the developer's glibc 2.44 host. Install the
native dependencies from the workflow plus `patchelf`, Python and Docker.

```sh
cargo build --locked --release
bash packaging/linux/build-appimage.sh target/release/grajek \
  /tmp/Grajek-THIRD-PARTY.txt /tmp/grajek-dist
```

The builder stages a fresh AppDir outside the checkout. It bundles exactly:

- libopus.so.0
- libxkbcommon.so.0
- libxkbcommon-x11.so.0
- libxcb-xkb.so.1

RPATH is `$ORIGIN/../lib` for Grajek and `$ORIGIN` for each library. AppRun is a
direct symlink, with no shell wrapper or LD_LIBRARY_PATH override. glibc, libgcc,
libxcb, Wayland and GPU/driver libraries stay on the host. The intentional host
exclusions are listed in `linux/check-appdir.py`; unresolved dependencies outside
that set fail the check. Desktop integration and XKB data remain host services.

appimagetool release tag 1.9.1 and type2-runtime tag 20251108 are pinned by SHA
in the manifest. The former's banner says `continuous build`/`8c8c91f`, despite
the fixed release URL; the file hash, not this banner, identifies the tool.
The builder passes `--runtime-file` explicitly, preventing a moving runtime
download. `APPIMAGE_EXTRACT_AND_RUN=1` avoids requiring FUSE while packaging.

The builder checks ELF GLIBC ≤2.35 and RPATH before and after packing. Then run
the **full** validation from a machine with Docker, outside the builder:

```sh
image=$(realpath /tmp/grajek-dist/Grajek-0.6.0-x86_64.AppImage)
extracted=$(mktemp -d /tmp/grajek-extracted-XXXXXXXX)
(cd "$extracted" && "$image" --appimage-extract)
python3 -I packaging/linux/check-appdir.py "$extracted/squashfs-root"
```

This mounts only the extracted AppDir read-only into a fresh `ubuntu:22.04` and
runs ldd with no network. No builder needs access to the host's Docker socket.
`--elf-only` is an intermediate check and does not close the container-ldd gate.
The clean container lacks libxcb.so.1; this is an intentional host dependency,
not permission to bundle it or assume a desktop is working.

The host allowlist was compared with AppImage/pkg2appimage's upstream excludelist
on 2026-11-03. The four bundled libraries are not on that list. Vulkan and
libwayland-egl are explicit host-driver/compositor exceptions from plan 004;
unrelated Xau/Xdmcp/bsd/md allowances were removed after the comparison.

## Windows Setup

Use native Windows with MSVC/SDK, CMake and **Inno Setup 6.7.1**. The target
rustflags request static CRT. Keep the original build output and check for
LNK4098; use dumpbin to reject VCRUNTIME140/MSVCP140 imports in Grajek and Deno.
The PyInstaller tree is recorded separately and still needs a clean VM.

From PowerShell, using a fresh directory outside the checkout:

```powershell
$tools = Join-Path $env:TEMP ('grajek-components-' + [guid]::NewGuid())
python3 -I packaging/check-components.py --platform windows --directory $tools --iscc-defines "$tools\iscc-defines.txt"
cargo build --locked --release 2>&1 | Tee-Object "$env:TEMP\grajek-build.log"
./packaging/windows/check-crt.ps1 -Grajek target/release/grajek.exe -ToolsDirectory $tools -BuildLog "$env:TEMP\grajek-build.log"
cargo about generate --locked --all-features --target x86_64-pc-windows-msvc -c packaging/about.toml packaging/about.hbs -o "$env:TEMP\Grajek-THIRD-PARTY.txt"
./packaging/windows/build-installer.ps1 -StageDirectory "$env:TEMP\grajek-stage" -ToolsDirectory $tools -Notices "$env:TEMP\Grajek-THIRD-PARTY.txt" -OutputDirectory dist -Version 0.6.0
```

The stage contains no tool executables. Inno downloads from upstream during
installation, then extracts into `{app}\tools`; its version marker is last.
yt-dlp uses the declarative `download` flag. Deno cannot: `[Files]` has no mirror
list, and GitHub answered 504 for its release asset while `dl.deno.land` served
the same release. A `PrepareToInstall` download page therefore tries the GitHub
URL first and then the manifest's `mirror` pin, each with its own SHA-256, and
stops with `DenoDownloadFailed` when both fail. One silent install on Windows
(with an unreachable primary) logged the failed attempt, downloaded the mirror,
and passed `Verification successful`; the clean-VM and offline cases stay open in
`004-native-checklist.md`.
The stable AppId must not change across releases. `{app}` defaults to
`%LOCALAPPDATA%\Programs\Grajek`, separate from `%LOCALAPPDATA%\Grajek` user data.
AppMutex stays alive while hidden in tray; updates and uninstall require Quit.

Build separate fixtures with `-BadYtDlpHash` and `-BadDenoHash`. On a clean test
account, `test-installer.ps1` accepts their paths plus the good Setup and checks
failure cleanup, correct versions, uninstall and state preservation. It refuses
an existing Grajek installation or state file. This is a smoke test, not F5.

## Release gates

`package.yml` is shared by packaging-related CI and release. The release resolves
an existing tag and creates only a draft with Setup, AppImage, portable archives
and SHA256SUMS. A runner does not close F5. This implementation task does not
push, tag, dispatch workflows, publish or create a PR.

Keep the native acceptance checklist open until the real systems, tray/media
sessions and audible Discord scenarios are recorded.
