import AppKit
import LevraBindings
import SwiftUI

struct ExplorerView: View {
    @Bindable var model: ExplorerModel
    @Environment(AppModel.self) private var app
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var toTrash: NodeInfo?
    @State private var outcome: CleanupOutcome?
    @State private var showsSystem = false
    @AppStorage("explorerMode") private var mode: Mode = .list
    @State private var zoomAnchor: UnitPoint = .center

    enum Mode: String {
        case list, map
    }

    var body: some View {
        PageScroll {
            PageHeader(title: "Explorateur", subtitle: subtitle) {
                Button { chooseFolder() } label: { ButtonLabel("Choisir un dossier", icon: .folderOpen) }
                    .buttonStyle(.app(.secondary))
                if model.updating {
                    // la mesure enregistrée s'affiche direct et se met à jour en douce
                    HStack(spacing: 6) {
                        Spinner(size: 13)
                        Text(model.measuredAt.map { "Mesure d'\($0.formatted(.relative(presentation: .named))), mise à jour…" } ?? "Mise à jour…")
                            .textStyle(.caption)
                    }
                    .transition(.opacity)
                }
                Button { Task { await model.scan(keepingCurrent: true) } } label: { ButtonLabel("Rafraîchir", icon: .refresh) }
                    .buttonStyle(.app(.secondary))
                    .keyboardShortcut("r", modifiers: .command)
                    .disabled(model.phase != .ready || model.updating)
            }
        } content: {
            switch model.phase {
            case .idle:
                start
            case let .scanning(files, bytes):
                ScanningState(title: "Mesure de \(model.rootName)…", icon: .folder, files: files, bytes: bytes) {
                    model.cancelScan()
                }
            case .ready:
                browser
            }
        }
        .animation(Motion.snappy, value: model.phase)
        // The home folder is measured as soon as the page opens.
        .task { await model.openDefault() }
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
            message: "Levra mesure chaque dossier une fois, puis tu navigues dedans pour voir ce qui prend de la place."
        ) {
            Button("Analyser mon dossier personnel") { Task { await model.scan() } }
                .buttonStyle(.app(.primary, size: .large))
            Button("Choisir un autre dossier…") { chooseFolder() }
                .buttonStyle(.app(.ghost))
            if let error = model.error {
                Text(error).textStyle(.secondary).foregroundStyle(Theme.danger)
            }
        }
    }

    private var browser: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                breadcrumb
                Spacer(minLength: 12)
                SegmentedControl(options: [(Mode.list, "Liste"), (.map, "Carte")], selection: $mode)
            }
            if let outcome {
                OutcomeBanner(outcome: outcome) { withAnimation(Motion.snappy) { self.outcome = nil } }
                    .transition(.lift(reduceMotion: reduceMotion))
            }
            if !model.denied.isEmpty {
                FullDiskAccessBanner(deniedCount: model.denied.count)
            }
            let all = model.children
            let children = all.filter { !model.isSystemFolder($0) }
            let system = all.filter(model.isSystemFolder)
            if mode == .map {
                TreemapView(
                    nodes: children,
                    onOpen: { node, anchor in
                        zoomAnchor = anchor
                        withAnimation(reduceMotion ? Motion.snappy : Motion.glide) { model.open(node) }
                    },
                    onTrash: { toTrash = $0 }
                )
                .id(model.current?.id)
                .transition(
                    reduceMotion
                        ? .opacity
                        : .asymmetric(
                            insertion: .scale(scale: 0.4, anchor: zoomAnchor).combined(with: .opacity),
                            removal: .opacity.animation(Motion.quick)
                        )
                )
                if !system.isEmpty {
                    Text("Les dossiers de macOS (\(Format.bytes(system.reduce(0) { $0 + $1.bytes }))) sont laissés hors de la carte.")
                        .textStyle(.caption)
                }
            } else {
            VStack(spacing: 0) {
                ForEach(Array(children.enumerated()), id: \.element.id) { index, node in
                    row(node, isLast: index == children.count - 1 && system.isEmpty)
                }
                if !system.isEmpty {
                    SystemFoldersRow(
                        bytes: system.reduce(0) { $0 + $1.bytes },
                        parentBytes: model.current?.bytes ?? 0,
                        expanded: showsSystem
                    ) {
                        withAnimation(Motion.snappy) { showsSystem.toggle() }
                    }
                    if showsSystem {
                        ForEach(Array(system.enumerated()), id: \.element.id) { index, node in
                            row(node, isLast: index == system.count - 1)
                                .padding(.leading, 16)
                                .transition(.lift(reduceMotion: reduceMotion))
                        }
                    }
                }
            }
            .card()
            .id(model.current?.id)
            }
        }
        .animation(Motion.snappy, value: mode)
    }

    private func row(_ node: NodeInfo, isLast: Bool) -> some View {
        NodeRow(
            node: node,
            parentBytes: model.current?.bytes ?? 0,
            path: model.path(of: node),
            isLast: isLast,
            onOpen: { model.open(node) },
            onTrash: { toTrash = node }
        )
    }

    private var breadcrumb: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 2) {
                ForEach(Array(model.trail.enumerated()), id: \.element.id) { index, node in
                    if index > 0 {
                        Icon(.arrowRight, size: 12).foregroundStyle(Theme.textTertiary)
                    }
                    let name = index == 0 && model.root.path == NSHomeDirectory()
                        ? "Dossier personnel"
                        : index == 0 ? model.root.path : node.name
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

/// macOS's own folders at the root of the disk, folded into one row.
private struct SystemFoldersRow: View {
    let bytes: UInt64
    let parentBytes: UInt64
    let expanded: Bool
    let onToggle: () -> Void

    var body: some View {
        CardRow(isLast: !expanded) {
            Button(action: onToggle) {
                HStack(spacing: 12) {
                    Icon(.system, size: 16).foregroundStyle(Theme.textTertiary)
                    VStack(alignment: .leading, spacing: 2) {
                        Text("macOS et système").textStyle(.body)
                        Text("Géré par macOS : rien à y faire soi-même.").textStyle(.caption)
                    }
                    Spacer(minLength: 12)
                    SizeBar(fraction: parentBytes > 0 ? Double(bytes) / Double(parentBytes) : 0)
                    Text(Format.bytes(bytes))
                        .textStyle(.bodyStrong)
                        .monospacedDigit()
                        .frame(width: 76, alignment: .trailing)
                    Icon(.arrowDown, size: 13)
                        .foregroundStyle(Theme.textTertiary)
                        .rotationEffect(.degrees(expanded ? 180 : 0))
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
        .accessibilityHint(expanded ? "Masque les dossiers de macOS" : "Affiche les dossiers de macOS")
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
