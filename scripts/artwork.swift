// Draws Levra's artwork from LevraMark:
//   - the app icon, as an .iconset following Apple's macOS icon template
//     (the tile is 824 points wide in a 1024-point canvas, over a soft
//     shadow), which scripts/build-app.sh turns into Levra.icns;
//   - the disk image background, with the layout that scripts/make-dmg.sh
//     gives dmgbuild, so that the icons land where the drawing expects them.
// Both scripts compile it together with LevraMark.swift.
//
//   artwork icon path/to/Levra.iconset
//   artwork dmg-background path/to/folder

import AppKit
import SwiftUI

@main
@MainActor
struct Artwork {
    static func main() throws {
        let arguments = Array(CommandLine.arguments.dropFirst())
        guard arguments.count == 2 else { usage() }
        let output = URL(fileURLWithPath: arguments[1])
        try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
        switch arguments[0] {
        case "icon": try icon(into: output)
        case "dmg-background": try diskImageBackground(into: output)
        default: usage()
        }
    }

    static func usage() -> Never {
        FileHandle.standardError.write(Data("usage : artwork icon dossier.iconset | artwork dmg-background dossier\n".utf8))
        exit(2)
    }

    static func icon(into iconset: URL) throws {
        for points in [16, 32, 128, 256, 512] {
            for scale in [1, 2] {
                let pixels = CGFloat(points * scale)
                let unit = pixels / 1024
                let icon = LevraMark(size: 824 * unit)
                    .shadow(color: .black.opacity(0.3), radius: 10 * unit, y: 10 * unit)
                    .frame(width: pixels, height: pixels)
                let name = scale == 1 ? "icon_\(points)x\(points).png" : "icon_\(points)x\(points)@2x.png"
                try render(icon, scale: 1, to: iconset.appending(path: name))
            }
        }
    }

    /// background.png and background@2x.png, which dmgbuild combines, and
    /// layout.json, where the window and the two icons go.
    static func diskImageBackground(into folder: URL) throws {
        try render(DiskImageBackground(), scale: 1, to: folder.appending(path: "background.png"))
        try render(DiskImageBackground(), scale: 2, to: folder.appending(path: "background@2x.png"))
        let layout: [String: [CGFloat]] = [
            "window": [DiskImageBackground.size.width, DiskImageBackground.size.height],
            "app": [DiskImageBackground.app.x, DiskImageBackground.app.y],
            "applications": [DiskImageBackground.applications.x, DiskImageBackground.applications.y],
        ]
        try JSONEncoder().encode(layout).write(to: folder.appending(path: "layout.json"))
    }

    static func render(_ view: some View, scale: CGFloat, to url: URL) throws {
        let renderer = ImageRenderer(content: view)
        renderer.scale = scale
        guard let image = renderer.cgImage,
              let png = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:])
        else {
            throw CocoaError(.fileWriteUnknown, userInfo: [NSFilePathErrorKey: url.path])
        }
        try png.write(to: url)
    }
}

/// The disk image window: the app on the left, Applications on the right,
/// an arrow from one to the other. The Finder draws the icons and their
/// names, in black or in white depending on the Mac's appearance, so the
/// background is a mid-tone indigo under the names, where both read.
struct DiskImageBackground: View {
    static let size = CGSize(width: 640, height: 400)
    /// Centres of the two icons, which are 128 points wide.
    static let app = CGPoint(x: 180, y: 220)
    static let applications = CGPoint(x: 460, y: 220)

    var body: some View {
        ZStack(alignment: .top) {
            LinearGradient(
                colors: [Color(red: 0x5E / 255, green: 0x66 / 255, blue: 0xBE / 255),
                         Color(red: 0x68 / 255, green: 0x70 / 255, blue: 0xC8 / 255)],
                startPoint: .top, endPoint: .bottom
            )
            VStack(spacing: 6) {
                Text("Installer Levra")
                    .font(.system(size: 22, weight: .semibold))
                    .tracking(-0.4)
                Text("Glisse l’app dans le dossier Applications.")
                    .font(.system(size: 13))
                    .opacity(0.9)
            }
            .foregroundStyle(.white)
            .padding(.top, 46)
            Arrow(from: CGPoint(x: Self.app.x + 86, y: Self.app.y),
                  to: CGPoint(x: Self.applications.x - 100, y: Self.applications.y))
                .stroke(.white.opacity(0.9), style: StrokeStyle(lineWidth: 4, lineCap: .round, lineJoin: .round, dash: [0.1, 12]))
            ArrowHead(at: CGPoint(x: Self.applications.x - 86, y: Self.applications.y))
                .stroke(.white.opacity(0.9), style: StrokeStyle(lineWidth: 4, lineCap: .round, lineJoin: .round))
        }
        .frame(width: Self.size.width, height: Self.size.height)
    }
}

/// A dotted line: the dash pattern draws one dot every 12 points.
private struct Arrow: Shape {
    let from: CGPoint
    let to: CGPoint

    func path(in rect: CGRect) -> Path {
        var path = Path()
        path.move(to: from)
        path.addLine(to: to)
        return path
    }
}

private struct ArrowHead: Shape {
    let at: CGPoint

    func path(in rect: CGRect) -> Path {
        var path = Path()
        path.move(to: CGPoint(x: at.x - 10, y: at.y - 11))
        path.addLine(to: at)
        path.addLine(to: CGPoint(x: at.x - 10, y: at.y + 11))
        return path
    }
}
