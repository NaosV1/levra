// Renders an .iconset of Levra's app icon from LevraMark, following Apple's
// macOS icon template: the tile is 824 points wide in a 1024-point canvas,
// over a soft shadow. scripts/build-app.sh compiles it with LevraMark.swift
// and turns the result into Levra.icns with iconutil.
//
//   make-icon path/to/Levra.iconset

import AppKit
import SwiftUI

@main
@MainActor
struct MakeIcon {
    static func main() throws {
        guard CommandLine.arguments.count == 2 else {
            FileHandle.standardError.write(Data("usage : make-icon dossier.iconset\n".utf8))
            exit(2)
        }
        let output = URL(fileURLWithPath: CommandLine.arguments[1])
        try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
        for points in [16, 32, 128, 256, 512] {
            try render(pixels: points, to: output.appending(path: "icon_\(points)x\(points).png"))
            try render(pixels: points * 2, to: output.appending(path: "icon_\(points)x\(points)@2x.png"))
        }
    }

    static func render(pixels: Int, to url: URL) throws {
        let canvas = CGFloat(pixels)
        let unit = canvas / 1024
        let icon = LevraMark(size: 824 * unit)
            .shadow(color: .black.opacity(0.3), radius: 10 * unit, y: 10 * unit)
            .frame(width: canvas, height: canvas)
        let renderer = ImageRenderer(content: icon)
        renderer.scale = 1
        guard let image = renderer.cgImage,
              let png = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:])
        else {
            throw CocoaError(.fileWriteUnknown, userInfo: [NSFilePathErrorKey: url.path])
        }
        try png.write(to: url)
    }
}
