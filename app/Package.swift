// swift-tools-version: 6.0
// The Rust core is linked through LevraFFI.xcframework and its
// generated bindings; run scripts/build-app.sh once to produce both.
import PackageDescription

let package = Package(
    name: "Levra",
    platforms: [.macOS(.v14)],
    targets: [
        .binaryTarget(name: "LevraFFI", path: "Frameworks/LevraFFI.xcframework"),
        .target(
            name: "LevraBindings",
            dependencies: ["LevraFFI"],
            path: "Sources/LevraBindings"
        ),
        .executableTarget(
            name: "Levra",
            dependencies: ["LevraBindings"],
            path: "Sources/Levra"
        ),
    ]
)
