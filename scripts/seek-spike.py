#!/usr/bin/env python3
"""Expose Songbird 0.6.1's existing mixer test harness in a disposable checkout.

No production dependency patches, Discord login, FFmpeg, or full-file downloads.
Run: python3 scripts/seek-spike.py (requires public YouTube network access).
"""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

root = Path(__file__).resolve().parents[1]
cargo_home = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo"))
source = next((cargo_home / "registry/src").glob("*/songbird-0.6.1"))
with tempfile.TemporaryDirectory(prefix="grajek-seek-spike-") as directory:
    workspace = Path(directory)
    songbird = workspace / "songbird"
    shutil.copytree(source, songbird)
    harness_files = ["config.rs", "driver/mod.rs", "driver/scheduler/live.rs", "driver/tasks/mixer/mod.rs"]
    for name in harness_files:
        path = songbird / "src" / name
        text = path.read_text()
        # Keep ordinary unit-test modules disabled; expose only the shipped harness.
        markers = [text.find(f"#[cfg(test)]\nmod {name}") for name in ["test {", "tests {"]]
        end = min((pos for pos in markers if pos >= 0), default=len(text))
        harness, tests = text[:end], text[end:]
        harness = harness.replace("cfg(test)", 'cfg(feature = "grajek-spike")')
        harness = harness.replace("cfg(not(test))", 'cfg(not(feature = "grajek-spike"))')
        harness = harness.replace("cfg!(not(test))", 'cfg!(not(feature = "grajek-spike"))')
        harness = harness.replace("cfg!(test)", 'cfg!(feature = "grajek-spike")')
        harness = harness.replace('all(test, feature = "driver")', 'all(feature = "grajek-spike", feature = "driver")')
        path.write_text(harness + tests)
    path = songbird / "Cargo.toml"
    manifest = path.read_text().replace("[features]\n", "[features]\ngrajek-spike = []\n", 1)
    manifest = manifest.replace('[dependencies.symphonia]\n', '[dependencies.symphonia]\nfeatures = ["mkv", "isomp4", "aac"]\n', 1)
    path.write_text(manifest)
    manifest = (root / "Cargo.toml").read_text().replace('name = "grajek"', 'name = "grajek-seek-spike"', 1)
    assert 'songbird = "=0.6.1"' in manifest, "spike must track the pinned Songbird version"
    manifest = manifest.replace('songbird = "=0.6.1"', 'songbird = { path = "songbird", features = ["grajek-spike"] }')
    # JSON quoting is valid TOML for this absolute path; no shell interpolation.
    import json
    manifest += '\n[[bin]]\nname = "seek-spike"\npath = ' + json.dumps(str(root / "scripts/seek-spike.rs")) + '\n'
    (workspace / "Cargo.toml").write_text(manifest)
    shutil.copyfile(root / "Cargo.lock", workspace / "Cargo.lock")
    result = subprocess.run(["cargo", "run", "--offline", "--manifest-path", str(workspace / "Cargo.toml"), "--bin", "seek-spike", "--", *sys.argv[1:]],
                            env={**os.environ, "CARGO_TARGET_DIR": str(root / "target")}, check=False)
    raise SystemExit(result.returncode)
