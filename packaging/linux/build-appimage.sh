#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 3 ]; then
  echo "Usage: $0 <grajek-binary> <Grajek-THIRD-PARTY.txt> <output-directory>" >&2
  exit 2
fi
root=$(cd -- "$(dirname -- "$0")/../.." && pwd)
binary=$(realpath "$1")
notices=$(realpath "$2")
output=$(realpath -m "$3")
test -s "$notices"
mkdir -p "$output"
work=$(mktemp -d "${TMPDIR:-/tmp}/grajek-appimage-XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT
appdir="$work/AppDir"
mkdir -p "$appdir/usr/bin" "$appdir/usr/lib" "$appdir/usr/share/licenses/grajek"
licenses="$appdir/usr/share/licenses/grajek"
cp "$binary" "$appdir/usr/bin/grajek"
ln -s usr/bin/grajek "$appdir/AppRun"
cp "$root/assets/linux/org.grajek.desktop.desktop" "$appdir/"
cp "$root/assets/brand/grajek-app-256.png" "$appdir/org.grajek.desktop.png"
ln -s org.grajek.desktop.png "$appdir/.DirIcon"
for size in 16 24 32 48 64 128 256 512 1024; do
  directory="$appdir/usr/share/icons/hicolor/${size}x${size}/apps"
  mkdir -p "$directory"
  cp "$root/assets/brand/grajek-app-$size.png" "$directory/org.grajek.desktop.png"
done
for library in libopus.so.0 libxkbcommon.so.0 libxkbcommon-x11.so.0 libxcb-xkb.so.1; do
  source=$(ldconfig -p | awk -v name="$library" '$1 == name && /x86-64/ && !found { print $NF; found=1 } END { if (!found) exit 1 }')
  cp -L "$source" "$appdir/usr/lib/$library"
  patchelf --set-rpath '$ORIGIN' "$appdir/usr/lib/$library"
done
patchelf --set-rpath '$ORIGIN/../lib' "$appdir/usr/bin/grajek"
cp "$root/LICENSE" "$licenses/Grajek-LICENSE.txt"
cp "$notices" "$licenses/Grajek-THIRD-PARTY.txt"
cp "$root/vendor/gpui-pre-linux/LICENSE-APACHE" "$licenses/gpui-LICENSE-APACHE.txt"
for package in libopus0 libxkbcommon0 libxkbcommon-x11-0 libxcb-xkb1; do
  cp "/usr/share/doc/$package/copyright" "$licenses/$package-copyright.txt"
done
cp "$root/packaging/linux/type2-runtime-LICENSE.txt" "$licenses/"
cp -R "$root/packaging/linux/runtime-licenses" "$licenses/"
python3 -I "$root/packaging/linux/check-appdir.py" "$appdir" --elf-only
python3 -I "$root/packaging/check-components.py" --platform build-tools --directory "$work/build-tools"
version=$(python3 -I -c 'import re,sys; print(re.search(r"^version\s*=\s*\"([^\"]+)\"", open(sys.argv[1]).read(), re.M)[1])' "$root/Cargo.toml")
destination="$output/Grajek-$version-x86_64.AppImage"
ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 "$work/build-tools/appimagetool/appimagetool" \
  --no-appstream --runtime-file "$work/build-tools/runtime/runtime" "$appdir" "$destination"
chmod +x "$destination"
mkdir "$work/extracted"
(cd "$work/extracted" && "$destination" --appimage-extract > /dev/null)
python3 -I "$root/packaging/linux/check-appdir.py" "$work/extracted/squashfs-root" --elf-only
echo "Container ldd validation of the extracted AppImage is still required."
echo "$destination"
