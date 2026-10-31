// swift-tools-version: 6.0
// The Rust core is linked through DebarrasFFI.xcframework and its
// generated bindings; run scripts/build-app.sh once to produce both.
import PackageDescription

let package = Package(
    name: "Debarras",
    platforms: [.macOS(.v14)],
    targets: [
        .binaryTarget(name: "DebarrasFFI", path: "Frameworks/DebarrasFFI.xcframework"),
        .target(
            name: "DebarrasBindings",
            dependencies: ["DebarrasFFI"],
            path: "Sources/DebarrasBindings"
        ),
        .executableTarget(
            name: "Debarras",
            dependencies: ["DebarrasBindings"],
            path: "Sources/Debarras"
        ),
    ]
)
