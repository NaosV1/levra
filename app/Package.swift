// swift-tools-version: 6.0
// The Rust core is linked through LevraFFI.xcframework and its
// generated bindings; run scripts/build-app.sh once to produce both.
import PackageDescription

let package = Package(
    name: "Levra",
    platforms: [.macOS(.v14)],
    dependencies: [
        // Updates outside the App Store. scripts/build-app.sh embeds the
        // framework in the bundle; scripts/appcast.sh signs each release.
        .package(url: "https://github.com/sparkle-project/Sparkle", exact: "2.10.1"),
    ],
    targets: [
        .binaryTarget(name: "LevraFFI", path: "Frameworks/LevraFFI.xcframework"),
        .target(
            name: "LevraBindings",
            dependencies: ["LevraFFI"],
            path: "Sources/LevraBindings"
        ),
        .executableTarget(
            name: "Levra",
            dependencies: ["LevraBindings", .product(name: "Sparkle", package: "Sparkle")],
            path: "Sources/Levra"
        ),
    ]
)
