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
binary="$(swift build -c release --package-path app --show-bin-path)/Levra"

# Drawn from the same LevraMark view as the mark inside the app.
echo "→ Icône"
swiftc -O -parse-as-library -o "$root/build/artwork" \
    scripts/artwork.swift app/Sources/Levra/Design/LevraMark.swift
rm -rf "$root/build/Levra.iconset"
"$root/build/artwork" icon "$root/build/Levra.iconset"
iconutil -c icns -o "$root/build/Levra.icns" "$root/build/Levra.iconset"

echo "→ Bundle"
rm -rf "$bundle"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
cp "$binary" "$bundle/Contents/MacOS/Levra"
cp app/Info.plist "$bundle/Contents/Info.plist"
cp "$root/build/Levra.icns" "$bundle/Contents/Resources/Levra.icns"
identity="${LEVRA_SIGN_IDENTITY:--}"
if [ "$identity" = "-" ]; then
    codesign --force --sign - --options runtime --entitlements app/Levra.entitlements "$bundle"
else
    # A secure timestamp is required for notarization.
    codesign --force --sign "$identity" --options runtime --timestamp \
        --entitlements app/Levra.entitlements "$bundle"
fi

echo "✓ $bundle"
if [ "${1:-}" = "--open" ]; then
    open "$bundle"
fi
