#!/usr/bin/env python3
"""Verify pinned upstream tools in a new directory outside the repository. Run with -I."""
import argparse
import hashlib
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import urllib.parse
import urllib.request
import zipfile
from pathlib import Path, PureWindowsPath

ROOT = Path(__file__).resolve().parents[1]


def verify_archive(path, pin):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    if path.stat().st_size != pin["size"] or digest.hexdigest() != pin["sha256"]:
        raise ValueError(f"Checksum or size mismatch: {path.name}")


def extract_zip(path, destination, unpacked_size):
    with zipfile.ZipFile(path) as archive:
        if sum(info.file_size for info in archive.infolist()) != unpacked_size:
            raise ValueError("Unexpected unpacked archive size")
        for info in archive.infolist():
            name = info.filename.replace("\\", "/")
            if name.startswith("/") or ".." in name.split("/") or PureWindowsPath(name).drive:
                raise ValueError(f"Unsafe archive path: {info.filename}")
        destination.mkdir()
        for info in archive.infolist():
            if stat.S_ISLNK(info.external_attr >> 16):
                continue
            target = destination / info.filename
            if info.is_dir():
                target.mkdir(parents=True, exist_ok=True)
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            with archive.open(info) as source, target.open("xb") as output:
                shutil.copyfileobj(source, output)
            if sys.platform != "win32" and (mode := (info.external_attr >> 16) & 0o777):
                target.chmod(mode)


class HttpsRedirects(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, url):
        if urllib.parse.urlsplit(url).scheme != "https":
            raise ValueError("Refusing a non-HTTPS download redirect")
        return super().redirect_request(request, response, code, message, headers, url)


def fetch(pin, path):
    if urllib.parse.urlsplit(pin["url"]).scheme != "https":
        raise ValueError("Pinned download must use HTTPS")
    opener = urllib.request.build_opener(HttpsRedirects())
    with opener.open(pin["url"], timeout=30) as response, path.open("xb") as output:
        size = 0
        while chunk := response.read(1024 * 1024):
            size += len(chunk)
            if size > pin["size"]:
                raise ValueError(f"Oversized download: {path.name}")
            output.write(chunk)
    verify_archive(path, pin)


def run(command, directory):
    environment = dict(os.environ, PATH="", HOME=str(directory), XDG_CONFIG_HOME=str(directory),
                       XDG_CACHE_HOME=str(directory), DENO_DIR=str(directory / "deno-cache"))
    result = subprocess.run([str(arg) for arg in command], cwd=directory, env=environment,
                            text=True, capture_output=True, timeout=30,
                            creationflags=subprocess.CREATE_NO_WINDOW if sys.platform == "win32" else 0)
    if result.returncode:
        raise ValueError(f"Component check failed ({result.returncode}): {Path(command[0]).name}")
    return result


def glibc_versions(directory):
    maximum = (0, 0)
    for path in directory.rglob("*"):
        if not path.is_file():
            continue
        with path.open("rb") as stream:
            if stream.read(4) != b"\x7fELF":
                continue
        result = subprocess.run(["objdump", "-T", str(path)], text=True, capture_output=True)
        for value in re.findall(r"GLIBC_([0-9]+(?:\.[0-9]+)+)", result.stdout):
            maximum = max(maximum, tuple(map(int, value.split("."))))
    return maximum


def has_ejs(output, version):
    prefix = "[debug] Optional libraries: "
    return any(token.strip() == f"yt_dlp_ejs-{version}"
               for line in output.splitlines() if line.startswith(prefix)
               for token in line[len(prefix):].split(","))


def check_components(manifest, platform, directory, defines):
    if (platform == "windows") != (sys.platform == "win32"):
        raise ValueError("Run component executables on their native platform")
    executables = {}
    for tool in ("yt-dlp", "deno"):
        pin = manifest[tool][platform]
        work = directory / tool
        work.mkdir()
        archive = work / "upstream.zip"
        fetch(pin, archive)
        unpacked = work / "unpacked"
        extract_zip(archive, unpacked, pin["unpacked_size"])
        name = ("yt-dlp.exe" if tool == "yt-dlp" else "deno.exe") if platform == "windows" else ("yt-dlp_linux" if tool == "yt-dlp" else "deno")
        executable = unpacked / name
        executables[tool] = executable
        version = run([executable, "--version"], directory).stdout.split()
        actual = version[1] if tool == "deno" and len(version) > 1 else version[0] if version else ""
        if actual != manifest[tool]["version"]:
            raise ValueError(f"Unexpected {tool} version: {actual}")
        print(f"{tool} {actual}: hash, sizes and native --version verified")
        if platform == "linux":
            print(f"{tool} maximum GLIBC: {'.'.join(map(str, glibc_versions(unpacked)))}")
    fixture = directory / "ejs-probe.json"
    fixture.write_text(json.dumps({"id": "ejs-probe", "title": "EJS offline probe", "url": "https://example.invalid/probe.mp3", "ext": "mp3", "extractor": "generic", "extractor_key": "Generic", "webpage_url": "https://example.invalid/probe.mp3"}))
    result = run([executables["yt-dlp"], "--ignore-config", "--no-plugin-dirs", "--no-js-runtimes", "-v", "--simulate", "--load-info-json", fixture], directory)
    if not has_ejs(result.stderr, manifest["yt-dlp"]["ejs"]):
        raise ValueError("Pinned yt-dlp-ejs library was not reported by the offline probe")
    print(f"yt-dlp-ejs {manifest['yt-dlp']['ejs']}: offline probe verified")
    (directory / "components.json").write_text(json.dumps({"yt-dlp": manifest["yt-dlp"]["version"], "yt-dlp-ejs": manifest["yt-dlp"]["ejs"], "deno": manifest["deno"]["version"]}) + "\n")
    if platform == "windows":
        values = {"ytdlp_version": manifest["yt-dlp"]["version"], "ejs_version": manifest["yt-dlp"]["ejs"],
                  "deno_version": manifest["deno"]["version"], "ytdlp_size": manifest["yt-dlp"]["windows"]["size"],
                  "deno_size": manifest["deno"]["windows"]["size"], "ytdlp_url": manifest["yt-dlp"]["windows"]["url"],
                  "deno_url": manifest["deno"]["windows"]["url"]}
        for language in ("en", "pl"):
            template = ROOT / f"packaging/windows/components-{language}.txt"
            (directory / template.name).write_text(template.read_text(encoding="utf-8").format(**values), encoding="utf-8-sig")
    if defines:
        if platform != "windows":
            raise ValueError("ISCC defines require Windows artifacts")
        values = {}
        for tool, prefix in (("yt-dlp", "YtDlp"), ("deno", "Deno")):
            pin = manifest[tool]["windows"]
            values.update({f"{prefix}Url": pin["url"], f"{prefix}Sha256": pin["sha256"], f"{prefix}ZipSize": pin["size"], f"{prefix}UnpackedSize": pin["unpacked_size"], f"{prefix}Version": manifest[tool]["version"]})
        values["EjsVersion"] = manifest["yt-dlp"]["ejs"]
        defines.write_text("\n".join(f"/D{key}={value}" for key, value in values.items()) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", required=True, choices=("linux", "windows", "build-tools"))
    parser.add_argument("--directory", required=True, type=Path)
    parser.add_argument("--iscc-defines", type=Path)
    args = parser.parse_args()
    directory = args.directory.resolve()
    if directory.is_relative_to(ROOT):
        raise ValueError("Downloads must be outside the repository")
    directory.mkdir(parents=True, exist_ok=True)
    if any(directory.iterdir()):
        raise ValueError("Download directory must be empty")
    manifest = json.loads((ROOT / "packaging/components.json").read_text())
    if args.platform == "build-tools":
        for tool, pin in manifest["appimage"].items():
            work = directory / tool
            work.mkdir()
            path = work / tool
            fetch(pin, path)
            path.chmod(0o755)
            print(f"{tool} {pin['version']}: SHA-256 verified")
    else:
        check_components(manifest, args.platform, directory, args.iscc_defines)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        raise SystemExit(str(error)) from error
