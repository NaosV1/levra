import AppKit
import ImageIO
import SwiftUI

struct PhotosView: View {
    let model: PhotosModel
    @Environment(AppModel.self) private var app
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var confirming = false

    var body: some View {
        PageScroll {
            PageHeader(title: "Photos similaires", subtitle: subtitle) {
                Button { Task { await model.find() } } label: {
                    ButtonLabel(model.phase == .idle ? "Rechercher" : "Relancer", icon: .refresh)
                }
                .buttonStyle(.app(.secondary))
                .keyboardShortcut("r", modifiers: .command)
                .disabled(model.phase != .ready && model.phase != .idle)
            }
        } content: {
            switch model.phase {
            case .idle:
                EmptyState(
                    icon: .layers,
                    title: "Photos qui se ressemblent",
                    message: "Rafales, photos reprises, captures enregistrées deux fois, copies redimensionnées : Levra compare ce qu'elles montrent, pas leurs octets. La photothèque Photos n'est pas touchée."
                ) {
                    Button("Rechercher les photos similaires") { Task { await model.find() } }
                        .buttonStyle(.app(.primary, size: .large))
                    if let error = model.error {
                        Text(error).textStyle(.secondary).foregroundStyle(Theme.danger)
                    }
                }
            case let .scanning(files, bytes):
                ScanningState(title: "Comparaison des photos…", icon: .layers, files: files, bytes: bytes) { model.cancelScan() }
            case .ready, .removing:
                results
            }
        }
        .safeAreaInset(edge: .bottom) {
            if model.phase == .ready || model.phase == .removing, !model.groups.isEmpty {
                ActionBar {
                    Text(Format.counted(model.selection.count, "photo")).textStyle(.secondary)
                    Text(Format.bytes(model.selectedBytes)).textStyle(.bodyStrong).monospacedDigit().contentTransition(.numericText())
                    Spacer()
                    if model.phase == .removing {
                        Spinner(size: 14)
                    }
                    Button("Sélection proposée") { withAnimation(Motion.quick) { model.selectProposed() } }
                        .buttonStyle(.app(.ghost))
                    Button { confirming = true } label: { ButtonLabel("Mettre à la corbeille", icon: .delete) }
                        .buttonStyle(.app(.danger))
                        .disabled(model.selection.isEmpty || model.phase == .removing)
                }
                .animation(Motion.snappy, value: model.selectedBytes)
                .transition(.move(edge: .bottom).combined(with: .opacity))
            }
        }
        .animation(Motion.snappy, value: model.phase)
        .confirmationDialog(
            "Mettre \(Format.counted(model.selection.count, "photo")) à la corbeille ?",
            isPresented: $confirming,
            titleVisibility: .visible
        ) {
            Button("Mettre à la corbeille", role: .destructive) {
                Task {
                    await model.remove()
                    app.refreshDisk()
                }
            }
        } message: {
            Text("\(Format.bytes(model.selectedBytes)). La meilleure photo de chaque groupe reste en place, et tout reste récupérable dans la corbeille.")
        }
    }

    private var subtitle: String {
        switch model.phase {
        case .idle: "Dans Images, Bureau, Téléchargements, Documents et Vidéos."
        case .scanning: "Lecture des images, rien n'est modifié."
        case .ready, .removing:
            "\(Format.counted(model.groups.count, "groupe")) · \(Format.bytes(model.removableBytes)) en trop"
        }
    }

    private var results: some View {
        VStack(alignment: .leading, spacing: 20) {
            if let outcome = model.outcome {
                OutcomeBanner(outcome: outcome) { withAnimation(Motion.snappy) { model.dismissOutcome() } }
                    .transition(.lift(reduceMotion: reduceMotion))
            }
            if model.groups.isEmpty {
                EmptyState(icon: .checkCircle, title: "Aucune photo similaire", message: "Chaque photo est unique.") {}
            }
            ForEach(Array(model.groups.enumerated()), id: \.element.id) { index, group in
                VStack(alignment: .leading, spacing: 10) {
                    HStack(spacing: 8) {
                        Text(Format.counted(group.photos.count, "photo")).textStyle(.bodyStrong)
                        Text("qui se ressemblent").textStyle(.secondary)
                        Spacer()
                        Text("\(Format.bytes(group.removableBytes)) en trop").textStyle(.label).monospacedDigit()
                    }
                    .padding(.horizontal, 4)
                    LazyVGrid(columns: [GridItem(.adaptive(minimum: 150, maximum: 220), spacing: 12)], spacing: 12) {
                        ForEach(Array(group.photos.enumerated()), id: \.element.id) { photoIndex, photo in
                            PhotoTile(
                                photo: photo,
                                best: photoIndex == 0,
                                isSelected: Binding(
                                    get: { model.selection.contains(photo.path) },
                                    set: { model.setSelected($0, photo: photo, in: group) }
                                )
                            )
                            .transition(.dismissRow(reduceMotion: reduceMotion))
                        }
                    }
                    .padding(12)
                    .card()
                }
                .staggeredAppear(index: index)
            }
        }
        .animation(Motion.snappy, value: model.groups)
    }
}

private struct PhotoTile: View {
    let photo: PhotosModel.Photo
    let best: Bool
    @Binding var isSelected: Bool
    @State private var hovering = false

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ZStack(alignment: .topLeading) {
                Thumbnail(path: photo.path)
                    .frame(height: 130)
                    .frame(maxWidth: .infinity)
                    .clipShape(RoundedRectangle(cornerRadius: Theme.radiusSmall))
                    .overlay(
                        RoundedRectangle(cornerRadius: Theme.radiusSmall)
                            .strokeBorder(isSelected ? Theme.danger.opacity(0.8) : Theme.border, lineWidth: isSelected ? 2 : 1)
                    )
                    // celles qui partent à la corbeille sont un peu éteintes
                    .saturation(isSelected ? 0.4 : 1)
                    .opacity(isSelected ? 0.75 : 1)
                    .scaleEffect(hovering ? 1.015 : 1)
                Checkbox(isOn: $isSelected)
                    .padding(8)
                if best {
                    Badge(text: "À garder", tone: .success)
                        .padding(8)
                        .frame(maxWidth: .infinity, alignment: .trailing)
                }
            }
            VStack(alignment: .leading, spacing: 2) {
                Text((photo.path as NSString).lastPathComponent).textStyle(.body).lineLimit(1).truncationMode(.middle)
                Text(details).textStyle(.caption).lineLimit(1)
            }
        }
        .contentShape(Rectangle())
        .onHover { hovering = $0 }
        .animation(Motion.hover, value: hovering)
        .animation(Motion.quick, value: isSelected)
        .onTapGesture(count: 2) { NSWorkspace.shared.open(URL(fileURLWithPath: photo.path)) }
        .contextMenu {
            Button("Ouvrir") { NSWorkspace.shared.open(URL(fileURLWithPath: photo.path)) }
            Button("Afficher dans le Finder") { SystemTrash.reveal(photo.path) }
        }
        .help(photo.path.abbreviatingHome)
        .accessibilityElement(children: .contain)
        .accessibilityLabel((photo.path as NSString).lastPathComponent)
    }

    private var details: String {
        var parts = [Format.bytes(photo.bytes)]
        if photo.pixels > 0 {
            parts.append(String(format: "%.1f Mpx", Double(photo.pixels) / 1_000_000))
        }
        if let modified = photo.modified {
            parts.append(modified.formatted(.dateTime.day().month().year()))
        }
        return parts.joined(separator: " · ")
    }
}

/// A picture's thumbnail, decoded off the main thread and kept for the
/// session.
private struct Thumbnail: View {
    let path: String
    @State private var image: NSImage?

    private static let cache = NSCache<NSString, NSImage>()

    var body: some View {
        ZStack {
            Theme.surfaceHover
            if let image {
                Image(nsImage: image)
                    .resizable()
                    .aspectRatio(contentMode: .fill)
                    .transition(.opacity)
            }
        }
        .task(id: path) {
            if let cached = Self.cache.object(forKey: path as NSString) {
                image = cached
                return
            }
            let path = path
            let loaded = await Task.detached(priority: .utility) { Self.load(path) }.value
            if let loaded {
                Self.cache.setObject(loaded, forKey: path as NSString)
                withAnimation(Motion.snappy) { image = loaded }
            }
        }
    }

    private nonisolated static func load(_ path: String) -> NSImage? {
        let url = URL(fileURLWithPath: path) as CFURL
        guard let source = CGImageSourceCreateWithURL(url, nil) else { return nil }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceThumbnailMaxPixelSize: 440,
            kCGImageSourceCreateThumbnailWithTransform: true,
        ]
        guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else { return nil }
        return NSImage(cgImage: image, size: NSSize(width: image.width, height: image.height))
    }
}
