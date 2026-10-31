#!/bin/sh
# Builds build/Débarras.app: Rust core → Swift bindings → XCFramework →
# SwiftUI app → bundle, signed ad hoc for this Mac.
#
#   scripts/build-app.sh          release build
#   scripts/build-app.sh --open   build, then launch the app
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

bindings="$root/build/bindings"
framework="$root/app/Frameworks/DebarrasFFI.xcframework"
generated="$root/app/Sources/DebarrasBindings/debarras_ffi.swift"
bundle="$root/build/Débarras.app"

# Same minimum as `platforms` in app/Package.swift. Without it, C code in
# Rust dependencies (BLAKE3's SIMD) targets the macOS doing the build.
export MACOSX_DEPLOYMENT_TARGET=14.0

echo "→ Cœur Rust"
cargo build --release -p debarras-ffi

echo "→ Bindings Swift"
rm -rf "$bindings"
cargo run --quiet --release -p debarras-ffi --bin uniffi-bindgen-swift -- \
    --swift-sources --headers --modulemap \
    --module-name debarras_ffiFFI --modulemap-filename module.modulemap \
    target/release/libdebarras_ffi.dylib "$bindings"
mkdir -p "$bindings/include" "$(dirname "$generated")"
mv "$bindings/debarras_ffiFFI.h" "$bindings/module.modulemap" "$bindings/include/"
mv "$bindings/debarras_ffi.swift" "$generated"

echo "→ XCFramework"
rm -rf "$framework"
mkdir -p "$(dirname "$framework")"
xcodebuild -create-xcframework \
    -library target/release/libdebarras_ffi.a \
    -headers "$bindings/include" \
    -output "$framework" >/dev/null

echo "→ App SwiftUI"
swift build -c release --package-path app
binary="$(swift build -c release --package-path app --show-bin-path)/Debarras"

echo "→ Bundle"
rm -rf "$bundle"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
cp "$binary" "$bundle/Contents/MacOS/Debarras"
cp app/Info.plist "$bundle/Contents/Info.plist"
codesign --force --sign - "$bundle"

echo "✓ $bundle"
if [ "${1:-}" = "--open" ]; then
    open "$bundle"
fi
