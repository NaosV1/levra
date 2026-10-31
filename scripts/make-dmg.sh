#!/bin/sh
# Packs the app into Levra's disk image: the app beside a link to
# Applications, over a background that says to drag one onto the other, with
# the app's icon on the volume. scripts/artwork.swift draws the background;
# dmgbuild writes the Finder's layout itself, without driving the Finder, so
# that it also works on a build server. Needs Python 3.10 or later.
#
#   scripts/make-dmg.sh build/Levra.app build/Levra.dmg
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
app="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
dmg="$2"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

echo "→ Image disque"
# Kept between builds; pip only checks it once installed.
venv="$root/build/dmgbuild"
[ -x "$venv/bin/python" ] || python3 -m venv "$venv"
"$venv/bin/pip" install --quiet --disable-pip-version-check --require-hashes \
    -r "$root/scripts/dmg/requirements.txt"

swiftc -O -parse-as-library -o "$work/artwork" \
    "$root/scripts/artwork.swift" "$root/app/Sources/Levra/Design/LevraMark.swift"
"$work/artwork" dmg-background "$work/background"

rm -f "$dmg"
"$venv/bin/dmgbuild" -s "$root/scripts/dmg/settings.py" \
    -D app="$app" \
    -D icon="$app/Contents/Resources/Levra.icns" \
    -D background="$work/background/background.png" \
    -D layout="$work/background/layout.json" \
    Levra "$dmg"
