import LevraBindings
import SwiftUI

/// The folder displayed as a map: each item a rectangle whose area is its
/// size, so what takes the space shows at a glance. Clicking a folder
/// zooms into it.
struct TreemapView: View {
    let nodes: [NodeInfo]
    let onOpen: (NodeInfo, UnitPoint) -> Void
    let onTrash: (NodeInfo) -> Void
    @State private var hovered: UInt64?

    var body: some View {
        GeometryReader { proxy in
            let size = proxy.size
            let tiles = Squarify.layout(nodes.filter { $0.bytes > 0 }, in: CGRect(origin: .zero, size: size))
            ZStack(alignment: .topLeading) {
                ForEach(Array(tiles.enumerated()), id: \.element.node.id) { index, tile in
                    TreemapTile(
                        node: tile.node,
                        rect: tile.rect,
                        hovered: hovered == tile.node.id,
                        index: index
                    )
                    .onHover { inside in
                        withAnimation(Motion.hover) {
                            if inside { hovered = tile.node.id } else if hovered == tile.node.id { hovered = nil }
                        }
                    }
                    .onTapGesture {
                        guard tile.node.kind == .directory, tile.node.hasChildren else { return }
                        // on zoome depuis la tuile cliquée, ça donne l'impression de rentrer dedans
                        let anchor = UnitPoint(
                            x: size.width > 0 ? tile.rect.midX / size.width : 0.5,
                            y: size.height > 0 ? tile.rect.midY / size.height : 0.5
                        )
                        onOpen(tile.node, anchor)
                    }
                    .contextMenu {
                        Button("Afficher dans le Finder") { SystemTrash.reveal(tile.node.path) }
                        if tile.node.removable {
                            Button("Mettre à la corbeille…", role: .destructive) { onTrash(tile.node) }
                        }
                    }
                }
            }
        }
        .frame(height: 460)
        .clipShape(RoundedRectangle(cornerRadius: Theme.radius))
        .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(Theme.border))
    }
}

private struct TreemapTile: View {
    let node: NodeInfo
    let rect: CGRect
    let hovered: Bool
    let index: Int
    @State private var visible = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private static let gap: CGFloat = 2

    var body: some View {
        let inner = rect.insetBy(dx: Self.gap / 2, dy: Self.gap / 2)
        ZStack(alignment: .topLeading) {
            RoundedRectangle(cornerRadius: 5)
                .fill(TreemapPalette.color(for: node).opacity(hovered ? 0.34 : 0.2))
            RoundedRectangle(cornerRadius: 5)
                .strokeBorder(TreemapPalette.color(for: node).opacity(hovered ? 0.7 : 0.28), lineWidth: 1)
            if inner.width > 64, inner.height > 34 {
                VStack(alignment: .leading, spacing: 1) {
                    Text(node.name)
                        .font(.system(size: inner.width > 160 && inner.height > 60 ? 13 : 11, weight: .medium))
                        .foregroundStyle(Theme.textPrimary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                    Text(Format.bytes(node.bytes))
                        .font(.system(size: 11).monospacedDigit())
                        .foregroundStyle(Theme.textSecondary)
                }
                .padding(7)
            }
        }
        .frame(width: max(inner.width, 0), height: max(inner.height, 0))
        .contentShape(RoundedRectangle(cornerRadius: 5))
        .scaleEffect(visible || reduceMotion ? 1 : 0.94, anchor: .center)
        .opacity(visible ? 1 : 0)
        .offset(x: inner.minX, y: inner.minY)
        .help("\(node.name) · \(Format.bytes(node.bytes))")
        .onAppear {
            // petite cascade, les gros d'abord vu qu'ils sont triés
            withAnimation(Motion.snappy.delay(Double(min(index, 24)) * 0.015)) { visible = true }
        }
        .accessibilityElement()
        .accessibilityLabel("\(node.name), \(Format.bytes(node.bytes))")
        .accessibilityAddTraits(node.kind == .directory ? .isButton : [])
    }
}

/// One hue per kind of content, muted: the map is read by area first.
enum TreemapPalette {
    static func color(for node: NodeInfo) -> Color {
        switch node.kind {
        case .directory: return Theme.accent
        case .smallFiles: return Theme.textTertiary
        case .file: break
        }
        let ext = (node.name as NSString).pathExtension.lowercased()
        if FileKinds.video.contains(ext) { return Color(light: 0x9333EA, dark: 0xB57BF2) }
        if FileKinds.image.contains(ext) { return Theme.success }
        if FileKinds.archive.contains(ext) { return Theme.warning }
        return Theme.textSecondary
    }
}

/// Extensions grouped by what they hold, shared by the map and the large
/// files filter.
enum FileKinds {
    static let video: Set<String> = ["mov", "mp4", "m4v", "mkv", "avi", "webm", "mts", "m2ts", "wmv", "flv", "hevc", "prores"]
    static let image: Set<String> = ["jpg", "jpeg", "png", "heic", "heif", "gif", "tiff", "tif", "raw", "dng", "cr2", "cr3", "nef", "arw", "webp", "psd"]
    static let archive: Set<String> = ["zip", "dmg", "pkg", "iso", "xip", "tar", "gz", "tgz", "xz", "7z", "rar", "bz2", "zst", "aar"]
    static let audio: Set<String> = ["mp3", "m4a", "wav", "aiff", "aif", "flac", "aac", "ogg", "caf", "logicx"]
}

/// Squarified treemap (Bruls, Huizing, van Wijk): rows of tiles whose
/// aspect ratio stays close to a square, so sizes compare by eye.
enum Squarify {
    struct Tile {
        let node: NodeInfo
        let rect: CGRect
    }

    static func layout(_ nodes: [NodeInfo], in bounds: CGRect) -> [Tile] {
        let total = nodes.reduce(0.0) { $0 + Double($1.bytes) }
        guard total > 0, bounds.width > 0, bounds.height > 0 else { return [] }
        let scale = Double(bounds.width * bounds.height) / total
        // les tuiles minuscules on les vire, illisibles et ça coûte cher en vues
        let items = nodes
            .sorted { $0.bytes > $1.bytes }
            .map { (node: $0, area: Double($0.bytes) * scale) }
            .filter { $0.area >= 16 }

        var tiles: [Tile] = []
        var remaining = bounds
        var row: [(node: NodeInfo, area: Double)] = []
        var index = 0
        while index < items.count {
            let item = items[index]
            let side = Double(min(remaining.width, remaining.height))
            if row.isEmpty || worst(row + [item], side: side) <= worst(row, side: side) {
                row.append(item)
                index += 1
            } else {
                remaining = place(row, in: remaining, into: &tiles)
                row = []
            }
        }
        if !row.isEmpty {
            _ = place(row, in: remaining, into: &tiles)
        }
        return tiles
    }

    /// Largest aspect ratio of a row laid along a side of this length.
    private static func worst(_ row: [(node: NodeInfo, area: Double)], side: Double) -> Double {
        let sum = row.reduce(0) { $0 + $1.area }
        guard sum > 0, side > 0 else { return .infinity }
        let largest = row.map(\.area).max() ?? 0
        let smallest = row.map(\.area).min() ?? 0
        return max(side * side * largest / (sum * sum), (sum * sum) / (side * side * smallest))
    }

    /// Lays a row along the shorter side, returns the space left.
    private static func place(_ row: [(node: NodeInfo, area: Double)], in rect: CGRect, into tiles: inout [Tile]) -> CGRect {
        let sum = row.reduce(0) { $0 + $1.area }
        if rect.width >= rect.height {
            let width = CGFloat(sum) / rect.height
            var y = rect.minY
            for item in row {
                let height = CGFloat(item.area) / width
                tiles.append(Tile(node: item.node, rect: CGRect(x: rect.minX, y: y, width: width, height: height)))
                y += height
            }
            return CGRect(x: rect.minX + width, y: rect.minY, width: rect.width - width, height: rect.height)
        } else {
            let height = CGFloat(sum) / rect.width
            var x = rect.minX
            for item in row {
                let width = CGFloat(item.area) / height
                tiles.append(Tile(node: item.node, rect: CGRect(x: x, y: rect.minY, width: width, height: height)))
                x += width
            }
            return CGRect(x: rect.minX, y: rect.minY + height, width: rect.width, height: rect.height - height)
        }
    }
}
