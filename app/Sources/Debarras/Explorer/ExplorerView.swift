import AppKit
import DebarrasBindings
import SwiftUI

struct ExplorerView: View {
    @Bindable var model: ExplorerModel
    @Environment(AppModel.self) private var app
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var toTrash: NodeInfo?
    @State private var outcome: CleanupOutcome?

    var body: some View {
        PageScroll {
            PageHeader(title: "Explorateur", subtitle: subtitle) {
                Button { chooseFolder() } label: { ButtonLabel("Choisir un dossier", icon: .folderOpen) }
                    .buttonStyle(.app(.secondary))
                Button { Task { await model.scan() } } label: { ButtonLabel("Rafraîchir", icon: .refresh) }
                    .buttonStyle(.app(.secondary))
                    .keyboardShortcut("r", modifiers: .command)
                    .disabled(model.phase != .ready)
            }
        } content: {
            switch model.phase {
            case .idle:
                start
            case let .scanning(files, bytes):
                ScanningState(title: "Mesure de \(model.root.path.abbreviatingHome)…", icon: .folder, files: files, bytes: bytes) {
                    model.cancelScan()
                }
            case .ready:
                browser
            }
        }
        .animation(Motion.snappy, value: model.phase)
        .confirmationDialog(
            "Mettre « \(toTrash?.name ?? "") » à la corbeille ?",
            isPresented: Binding(get: { toTrash != nil }, set: { if !$0 { toTrash = nil } }),
            titleVisibility: .visible
        ) {
            Button("Mettre à la corbeille", role: .destructive) {
                guard let node = toTrash else { return }
                Task {
                    let result = await model.trash([node])
                    withAnimation(Motion.snappy) { outcome = result }
                    app.refreshDisk()
                }
            }
        } message: {
            Text("\(Format.bytes(toTrash?.bytes ?? 0)). Récupérable tant que la corbeille n'est pas vidée.")
        }
    }

    private var subtitle: String {
        switch model.phase {
        case .idle: "Mesure un dossier une fois, puis navigue instantanément."
        case .scanning: "Lecture du disque, rien n'est modifié."
        case .ready:
            model.current.map { "\(Format.bytes($0.bytes)) · \(Format.count($0.items)) éléments" } ?? ""
        }
    }

    private var start: some View {
        EmptyState(
            icon: .folder,
            title: "Où va l'espace ?",
            message: "Débarras mesure la taille réelle de chaque dossier, comme du, en parallèle."
        ) {
            Button("Analyser mon dossier personnel") { Task { await model.scan() } }
                .buttonStyle(.app(.primary))
            Button("Choisir un dossier…") { chooseFolder() }
                .buttonStyle(.app(.secondary))
            if let error = model.error {
                Text(error).textStyle(.secondary).foregroundStyle(Theme.danger)
            }
        }
    }

    private var browser: some View {
        VStack(alignment: .leading, spacing: 16) {
            breadcrumb
            if let outcome {
                OutcomeBanner(outcome: outcome) { withAnimation(Motion.snappy) { self.outcome = nil } }
                    .transition(.lift(reduceMotion: reduceMotion))
            }
            if !model.denied.isEmpty {
                FullDiskAccessBanner(deniedCount: model.denied.count)
            }
            let children = model.children
            VStack(spacing: 0) {
                ForEach(Array(children.enumerated()), id: \.element.id) { index, node in
                    NodeRow(
                        node: node,
                        parentBytes: model.current?.bytes ?? 0,
                        path: model.path(of: node),
                        isLast: index == children.count - 1,
                        onOpen: { model.open(node) },
                        onTrash: { toTrash = node }
                    )
                }
            }
            .card()
            .id(model.current?.id)
        }
    }

    private var breadcrumb: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 2) {
                ForEach(Array(model.trail.enumerated()), id: \.element.id) { index, node in
                    if index > 0 {
                        Icon(.arrowRight, size: 12).foregroundStyle(Theme.textTertiary)
                    }
                    let name = index == 0 ? model.root.path.abbreviatingHome : node.name
                    if index == model.trail.count - 1 {
                        Text(name).textStyle(.bodyStrong).padding(.horizontal, 8)
                    } else {
                        Button(name) { model.goBack(to: index) }
                            .buttonStyle(.app(.ghost, size: .small))
                    }
                }
            }
        }
    }

    private func chooseFolder() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.directoryURL = model.root
        panel.prompt = "Analyser"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        Task { await model.choose(url) }
    }
}

private struct NodeRow: View {
    let node: NodeInfo
    let parentBytes: UInt64
    let path: String?
    let isLast: Bool
    let onOpen: () -> Void
    let onTrash: () -> Void

    private var opens: Bool { node.kind == .directory && node.hasChildren }

    var body: some View {
        CardRow(isLast: isLast) {
            HStack(spacing: 12) {
                Icon(icon, size: 16).foregroundStyle(node.kind == .directory ? Theme.textPrimary : Theme.textTertiary)
                Text(node.name).textStyle(.body).lineLimit(1).truncationMode(.middle)
                if node.kind != .file {
                    Text("\(Format.count(node.items))").textStyle(.caption).monospacedDigit()
                }
                Spacer(minLength: 12)
                HoverReveal {
                    HStack(spacing: 2) {
                        if let path {
                            IconButton(icon: .arrowUpRight, help: "Afficher dans le Finder") { SystemTrash.reveal(path) }
                        }
                        if node.removable {
                            IconButton(icon: .delete, help: "Mettre à la corbeille", tint: Theme.danger, action: onTrash)
                        }
                    }
                }
                SizeBar(fraction: parentBytes > 0 ? Double(node.bytes) / Double(parentBytes) : 0)
                Text(Format.bytes(node.bytes))
                    .textStyle(.bodyStrong)
                    .monospacedDigit()
                    .frame(width: 76, alignment: .trailing)
                Icon(.arrowRight, size: 13)
                    .foregroundStyle(Theme.textTertiary)
                    .opacity(opens ? 1 : 0)
            }
        }
        .onTapGesture { if opens { onOpen() } }
        .accessibilityElement(children: .contain)
        .accessibilityAction(named: "Ouvrir") { if opens { onOpen() } }
        .accessibilityAction(named: "Afficher dans le Finder") { if let path { SystemTrash.reveal(path) } }
        .accessibilityAction(named: "Mettre à la corbeille") { if node.removable { onTrash() } }
    }

    private var icon: Papicon {
        switch node.kind {
        case .directory: .folder
        case .file: .file
        case .smallFiles: .layers
        }
    }
}
