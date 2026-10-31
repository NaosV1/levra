#!/bin/sh
# Builds Levra for distribution outside the Mac App Store: signed with a
# Developer ID certificate, notarized by Apple and stapled, in a disk image
# that installs by dragging the app to Applications.
#
#   LEVRA_NOTARY_PROFILE=levra scripts/release.sh
#
# Needs, once on this Mac:
#   - a "Developer ID Application" certificate in the keychain
#     (Xcode › Settings › Accounts › Manage Certificates › +);
#   - notarization credentials saved under a profile name, with an
#     app-specific password made at account.apple.com:
#       xcrun notarytool store-credentials levra --apple-id <email> --team-id <team>
#
# LEVRA_SIGN_IDENTITY picks the certificate when there are several.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

profile="${LEVRA_NOTARY_PROFILE:?nom du profil créé avec xcrun notarytool store-credentials}"
identity="${LEVRA_SIGN_IDENTITY:-$(security find-identity -v -p codesigning | sed -n 's/.*"\(Developer ID Application: .*\)"$/\1/p' | head -n 1)}"
if [ -z "$identity" ]; then
    echo "Aucun certificat « Developer ID Application » dans le trousseau :" >&2
    echo "crée-le dans Xcode › Réglages › Comptes › Gérer les certificats." >&2
    exit 1
fi

app="$root/build/Levra.app"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# Waits for Apple's verdict; on refusal, prints why and stops.
notarize() {
    echo "→ Notarisation de $(basename "$1") (quelques minutes)"
    result=$(xcrun notarytool submit "$1" --keychain-profile "$profile" --wait --output-format json)
    status=$(printf '%s' "$result" | plutil -extract status raw -o - -)
    if [ "$status" != "Accepted" ]; then
        id=$(printf '%s' "$result" | plutil -extract id raw -o - -)
        echo "Notarisation refusée ($status) :" >&2
        xcrun notarytool log "$id" --keychain-profile "$profile" >&2
        exit 1
    fi
}

echo "→ Build signé avec « $identity »"
LEVRA_SIGN_IDENTITY="$identity" scripts/build-app.sh
codesign --verify --strict "$app"
# Set by build-app.sh: major.minor from app/Info.plist, then the build number.
version=$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$app/Contents/Info.plist")
dmg="$root/build/Levra-$version.dmg"

# The app is notarized and stapled on its own first, so that it also opens
# offline once copied out of the disk image.
ditto -c -k --keepParent "$app" "$work/app.zip"
notarize "$work/app.zip"
xcrun stapler staple "$app"

scripts/make-dmg.sh "$app" "$dmg"
codesign --sign "$identity" --timestamp "$dmg"
notarize "$dmg"
xcrun stapler staple "$dmg"

spctl --assess --type open --context context:primary-signature --verbose "$dmg"
echo "✓ $dmg"
