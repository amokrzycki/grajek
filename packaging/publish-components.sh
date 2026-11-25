#!/usr/bin/env bash
# usage: packaging/publish-components.sh <private-key.pem> <output-dir>
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

if [ "$#" -ne 2 ]; then
  echo "usage: packaging/publish-components.sh <private-key.pem> <output-dir>" >&2
  exit 2
fi

key=$1
out=$2
pub=${COMPONENTS_PUB:-packaging/components.pub}

serial=$(python3 -I -c 'import json,sys; print(json.load(sys.stdin).get("serial", 0))' < packaging/components.json)
head_manifest=$(git show HEAD:packaging/components.json 2>/dev/null || echo '{}')
head_serial=$(printf '%s' "$head_manifest" | python3 -I -c 'import json,sys; print(json.load(sys.stdin).get("serial", 0))')
if [ "$serial" -le "$head_serial" ]; then
  echo "error: serial $serial in packaging/components.json must be greater than HEAD's $head_serial" >&2
  exit 1
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

echo "note: checking Linux artifacts; run check-components.py --platform windows on Windows" >&2
python3 -I packaging/check-components.py --platform linux --directory "$tmp"

mkdir -p "$out"
cp packaging/components.json "$out/components.json"
openssl pkeyutl -sign -rawin -inkey "$key" -in "$out/components.json" -out "$out/components.json.sig"
sig_size=$(wc -c < "$out/components.json.sig")
if [ "$sig_size" -ne 64 ]; then
  echo "error: signature is $sig_size bytes, expected 64" >&2
  exit 1
fi

pub_hex=$(cat "$pub")
if ! [[ $pub_hex =~ ^[0-9a-f]{64}$ ]]; then
  echo "error: $pub must contain exactly 64 lowercase hex characters" >&2
  exit 1
fi
printf '302a300506032b6570032100%s' "$pub_hex" | xxd -r -p > "$tmp/key.der"
openssl pkeyutl -verify -rawin -pubin -keyform DER -inkey "$tmp/key.der" \
  -in "$out/components.json" -sigfile "$out/components.json.sig"

if ! prerelease=$(gh release view components --json isPrerelease --jq .isPrerelease 2>/dev/null); then
  echo "error: GitHub release 'components' does not exist; create it with:" >&2
  echo 'gh release create components --prerelease --title "Playback components" --notes "Signed component manifest"' >&2
  exit 1
fi
if [ "$prerelease" != true ]; then
  echo "error: release 'components' must be a prerelease" >&2
  exit 1
fi

echo "upload with:"
echo "gh release upload components \"$out/components.json\" \"$out/components.json.sig\" --clobber"
