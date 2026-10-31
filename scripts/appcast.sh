#!/bin/sh
# Writes the appcast that Levra's updater reads: SUFeedURL in the app's
# Info.plist points at this file in the latest GitHub release. It offers
# that release's disk image, signed with Levra's Sparkle key: read from
# SPARKLE_PRIVATE_KEY on GitHub, from the keychain on this Mac (where
# generate_keys saved it under the "levra" account).
#
#   scripts/appcast.sh build/Levra.app build/Levra-0.2.0.dmg > build/appcast.xml
set -eu

app="$1"
dmg="$2"
root=$(cd "$(dirname "$0")/.." && pwd)
# Shipped with the Sparkle package that app/Package.swift pins.
sign_update="$root/app/.build/artifacts/sparkle/Sparkle/bin/sign_update"

read_info() {
    /usr/libexec/PlistBuddy -c "Print :$1" "$app/Contents/Info.plist"
}
version=$(read_info CFBundleShortVersionString)
build=$(read_info CFBundleVersion)
minimum=$(read_info LSMinimumSystemVersion)
# https://github.com/<owner>/<repo>/releases/latest/download/appcast.xml
releases=$(read_info SUFeedURL | sed 's|/latest/download/appcast.xml$||')

# Prints: sparkle:edSignature="…" length="…"
if [ -n "${SPARKLE_PRIVATE_KEY:-}" ]; then
    signature=$(printf '%s' "$SPARKLE_PRIVATE_KEY" | "$sign_update" --ed-key-file - "$dmg")
else
    signature=$("$sign_update" --account levra "$dmg")
fi

cat <<EOF
<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>Levra</title>
    <item>
      <title>Levra $version</title>
      <pubDate>$(LC_ALL=C date -u '+%a, %d %b %Y %H:%M:%S +0000')</pubDate>
      <sparkle:version>$build</sparkle:version>
      <sparkle:shortVersionString>$version</sparkle:shortVersionString>
      <sparkle:minimumSystemVersion>$minimum</sparkle:minimumSystemVersion>
      <sparkle:fullReleaseNotesLink>$releases/tag/v$version</sparkle:fullReleaseNotesLink>
      <enclosure url="$releases/download/v$version/$(basename "$dmg")" type="application/octet-stream" $signature/>
    </item>
  </channel>
</rss>
EOF
