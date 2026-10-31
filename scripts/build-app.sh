#!/bin/sh
# Builds build/Levra.app: Rust core → Swift bindings → XCFramework →
# SwiftUI app → bundle, signed with the hardened runtime that notarization
# requires: ad hoc for this Mac, or with the certificate named in
# LEVRA_SIGN_IDENTITY (scripts/release.sh sets it).
#
#   scripts/build-app.sh          release build
#   scripts/build-app.sh --open   build, then launch the app
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

bindings="$root/build/bindings"
framework="$root/app/Frameworks/LevraFFI.xcframework"
generated="$root/app/Sources/LevraBindings/levra_ffi.swift"
bundle="$root/build/Levra.app"

# Same minimum as `platforms` in app/Package.swift. Without it, C code in
# Rust dependencies (BLAKE3's SIMD) targets the macOS doing the build.
export MACOSX_DEPLOYMENT_TARGET=14.0

echo "→ Cœur Rust"
cargo build --release -p levra-ffi

echo "→ Bindings Swift"
rm -rf "$bindings"
cargo run --quiet --release -p levra-ffi --bin uniffi-bindgen-swift -- \
    --swift-sources --headers --modulemap \
    --module-name levra_ffiFFI --modulemap-filename module.modulemap \
    target/release/liblevra_ffi.dylib "$bindings"
mkdir -p "$bindings/include" "$(dirname "$generated")"
mv "$bindings/levra_ffiFFI.h" "$bindings/module.modulemap" "$bindings/include/"
mv "$bindings/levra_ffi.swift" "$generated"

echo "→ XCFramework"
rm -rf "$framework"
mkdir -p "$(dirname "$framework")"
xcodebuild -create-xcframework \
    -library target/release/liblevra_ffi.a \
    -headers "$bindings/include" \
    -output "$framework" >/dev/null

echo "→ App SwiftUI"
swift build -c release --package-path app
products="$(swift build -c release --package-path app --show-bin-path)"

# Drawn from the same LevraMark view as the mark inside the app.
echo "→ Icône"
swiftc -O -parse-as-library -o "$root/build/artwork" \
    scripts/artwork.swift app/Sources/Levra/Design/LevraMark.swift
rm -rf "$root/build/Levra.iconset"
"$root/build/artwork" icon "$root/build/Levra.iconset"
iconutil -c icns -o "$root/build/Levra.icns" "$root/build/Levra.iconset"

echo "→ Bundle"
rm -rf "$bundle"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources" "$bundle/Contents/Frameworks"
cp "$products/Levra" "$bundle/Contents/MacOS/Levra"
# SwiftPM expects frameworks next to the executable; a bundle keeps them
# in Contents/Frameworks.
install_name_tool -add_rpath @executable_path/../Frameworks "$bundle/Contents/MacOS/Levra"
cp app/Info.plist "$bundle/Contents/Info.plist"
# The build number is the number of commits, which only grows on main (CI
# needs the full history): Sparkle offers an update only when it is
# higher. The version shown is app/Info.plist's major.minor, then that
# number: 0.1.8, 0.1.9…
build=$(git rev-list --count HEAD)
release=$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" app/Info.plist)
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $build" -c "Set :CFBundleShortVersionString $release.$build" \
    "$bundle/Contents/Info.plist"
cp "$root/build/Levra.icns" "$bundle/Contents/Resources/Levra.icns"
# Sparkle's XPC services are for sandboxed apps; Levra is not one.
sparkle="$bundle/Contents/Frameworks/Sparkle.framework"
ditto "$products/Sparkle.framework" "$sparkle"
rm -rf "$sparkle/XPCServices" "$sparkle/Versions/B/XPCServices"

# Inside out, as codesign requires: Sparkle's helpers, Sparkle, the app.
identity="${LEVRA_SIGN_IDENTITY:--}"
sign() {
    if [ "$identity" = "-" ]; then
        codesign --force --sign - --options runtime "$@"
    else
        # A secure timestamp is required for notarization.
        codesign --force --sign "$identity" --options runtime --timestamp "$@"
    fi
}
sign "$sparkle/Versions/B/Autoupdate"
sign "$sparkle/Versions/B/Updater.app"
sign "$sparkle"
entitlements=app/Levra.entitlements
if [ "$identity" = "-" ]; then
    # The hardened runtime loads only frameworks signed by the app's team,
    # and an ad hoc signature has none: test builds may load Sparkle all
    # the same. Builds signed with Developer ID keep the check.
    entitlements="$root/build/adhoc.entitlements"
    cp app/Levra.entitlements "$entitlements"
    /usr/libexec/PlistBuddy -c "Add :com.apple.security.cs.disable-library-validation bool true" "$entitlements"
fi
sign --entitlements "$entitlements" "$bundle"

echo "✓ $bundle"
if [ "${1:-}" = "--open" ]; then
    open "$bundle"
fi
