"""Check every shipped ELF and resolve dependencies in a clean Ubuntu 22.04 container."""
import re
import subprocess
import sys
from pathlib import Path

HOST_LIBRARIES = {"libc.so.6", "libm.so.6", "libgcc_s.so.1", "libpthread.so.0", "libdl.so.2",
                  "librt.so.1", "ld-linux-x86-64.so.2", "libxcb.so.1", "libX11.so.6",
                  "libwayland-client.so.0", "libwayland-egl.so.1", "libEGL.so.1", "libvulkan.so.1"}
LIBRARIES = {"libopus.so.0", "libxkbcommon.so.0", "libxkbcommon-x11.so.0", "libxcb-xkb.so.1"}


def main():
    root = Path(sys.argv[1]).resolve()
    if {p.name for p in (root / "usr/lib").iterdir()} != LIBRARIES:
        raise SystemExit("AppDir must contain exactly the four approved shared libraries")
    if (root / "AppRun").readlink() != Path("usr/bin/grajek"):
        raise SystemExit("AppRun must be a direct symlink to usr/bin/grajek")
    elfs = []
    for path in root.rglob("*"):
        if path.is_symlink() or not path.is_file():
            continue
        if path.name in {"yt-dlp", "yt-dlp_linux", "yt-dlp.exe", "deno", "deno.exe", "components.json"}:
            raise SystemExit(f"Playback component must not be bundled: {path}")
        with path.open("rb") as stream:
            if stream.read(4) != b"\x7fELF":
                continue
        elfs.append(path)
        output = subprocess.run(["objdump", "-T", str(path)], text=True, capture_output=True, check=True).stdout
        versions = [tuple(map(int, version.split("."))) for version in re.findall(r"GLIBC_([0-9]+(?:\.[0-9]+)+)", output)]
        if any(version > (2, 35) for version in versions):
            raise SystemExit(f"{path.name} requires GLIBC > 2.35; build on Ubuntu 22.04")
        expected = "$ORIGIN/../lib" if path == root / "usr/bin/grajek" else "$ORIGIN"
        dynamic = subprocess.check_output(["objdump", "-p", str(path)], text=True)
        found = re.search(r"^\s*(?:RUNPATH|RPATH)\s+(.+)$", dynamic, re.M)
        rpath = found[1].strip() if found else ""
        if rpath != expected:
            raise SystemExit(f"Unexpected RPATH in {path.name}: {rpath}")
    if len(elfs) != 5:
        raise SystemExit("Expected Grajek and four shared-library ELF files")
    if "--elf-only" in sys.argv[2:]:
        print("ELF GLIBC <= 2.35 and RPATH passed; container ldd still required")
        return
    result = subprocess.run(["docker", "run", "--rm", "--network", "none", "-v", f"{root}:/app:ro", "ubuntu:22.04",
                             "sh", "-c", 'for file in /app/usr/bin/grajek /app/usr/lib/*.so*; do echo "$file"; ldd "$file" || exit $?; done'],
                            text=True, capture_output=True)
    print(result.stdout)
    if result.returncode or re.search(r"version .*not found", result.stdout + result.stderr):
        raise SystemExit("Dependency validation container failed: " + result.stderr)
    missing = set(re.findall(r"(\S+)\s+=>\s+not found", result.stdout))
    if unexpected := missing - HOST_LIBRARIES:
        raise SystemExit("Unresolved dependencies outside the host exclusion list: " + ", ".join(sorted(unexpected)))
    print("ELF GLIBC <= 2.35, RPATH and clean Ubuntu 22.04 dependency checks passed")


if __name__ == "__main__":
    main()
