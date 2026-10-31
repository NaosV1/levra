import AppKit
import DebarrasBindings
import SwiftUI

struct DuplicatesView: View {
    let model: DuplicatesModel
    @Environment(AppModel.self) private var app
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var confirming = false

    var body: some View {
        PageScroll {
            PageHeader(title: "Doublons", subtitle: subtitle) {
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
                    icon: .copy,
                    title: "Fichiers en double",
                    message: "Compare le contenu des fichiers de ton dossier personnel. Les copies clonées par le Finder, qui ne prennent pas de place, sont écartées."
                ) {
                    Button("Rechercher les doublons") { Task { await model.find() } }
                        .buttonStyle(.app(.primary))
                    if let error = model.error {
                        Text(error).textStyle(.secondary).foregroundStyle(Theme.danger)
                    }
                }
            case let .scanning(files, bytes):
                ScanningState(title: "Comparaison des fichiers…", files: files, bytes: bytes) { model.cancelScan() }
            case .ready, .removing:
                results
            }
        }
        .safeAreaInset(edge: .bottom) {
            if model.phase == .ready || model.phase == .removing, !model.groups.isEmpty {
                ActionBar {
                    Text("\(model.selection.count) copie(s)").textStyle(.secondary)
                    Text(Format.bytes(model.selectedBytes)).textStyle(.bodyStrong).monospacedDigit().contentTransition(.numericText())
                    Spacer()
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
            "Mettre \(model.selection.count) copie(s) à la corbeille ?",
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
            Text("\(Format.bytes(model.selectedBytes)) libérés. Un exemplaire de chaque fichier reste en place.")
        }
    }

    private var subtitle: String {
        switch model.phase {
        case .idle: "Fichiers identiques, et l'espace que leurs copies libéreraient vraiment."
        case .scanning: "Lecture des fichiers, rien n'est modifié."
        case .ready, .removing:
            "\(model.groups.count) groupe(s) · \(Format.bytes(model.reclaimableBytes)) récupérables"
        }
    }

    private var results: some View {
        VStack(alignment: .leading, spacing: 16) {
            if let outcome = model.outcome {
                OutcomeBanner(outcome: outcome) { withAnimation(Motion.snappy) { model.dismissOutcome() } }
                    .transition(.lift(reduceMotion: reduceMotion))
            }
            if !model.denied.isEmpty {
                FullDiskAccessBanner(deniedCount: model.denied.count)
            }
            if model.cloneGroups > 0 {
                Banner(
                    tone: .neutral,
                    icon: .info,
                    title: "\(model.cloneGroups) groupe(s) de copies clonées ignorés",
                    message: "Elles partagent leurs données avec l'original : les supprimer ne libérerait rien."
                )
            }
            if model.groups.isEmpty {
                EmptyState(icon: .checkCircle, title: "Aucun doublon", message: "Rien qui prenne de la place en double.") {}
            }
            ForEach(Array(model.groups.enumerated()), id: \.element.id) { index, group in
                VStack(alignment: .leading, spacing: 8) {
                    GroupHeader(group: group)
                    VStack(spacing: 0) {
                        ForEach(Array(group.files.enumerated()), id: \.element.path) { fileIndex, file in
                            DuplicateRow(
                                file: file,
                                size: group.size,
                                isSelected: binding(for: file, in: group),
                                isLast: fileIndex == group.files.count - 1
                            )
                        }
                    }
                    .card()
                }
                .staggeredAppear(index: index)
            }
        }
        .animation(Motion.snappy, value: model.groups.map(\.id))
    }

    private func binding(for file: DuplicateFileInfo, in group: DuplicateGroupInfo) -> Binding<Bool> {
        Binding(
            get: { model.selection.contains(file.path) },
            set: { model.setSelected($0, file: file, in: group) }
        )
    }
}

private struct GroupHeader: View {
    let group: DuplicateGroupInfo

    var body: some View {
        HStack(spacing: 8) {
            AppIcon(path: group.files.first?.path ?? "", size: 16)
            Text(name).textStyle(.bodyStrong).lineLimit(1)
            Text("\(group.files.count) × \(Format.bytes(group.size))").textStyle(.caption).monospacedDigit()
            Spacer()
            Text("\(Format.bytes(group.reclaimable)) récupérables").textStyle(.label).monospacedDigit()
        }
        .padding(.horizontal, 4)
    }

    private var name: String {
        group.files.first.map { URL(fileURLWithPath: $0.path).lastPathComponent } ?? ""
    }
}

private struct DuplicateRow: View {
    let file: DuplicateFileInfo
    let size: UInt64
    @Binding var isSelected: Bool
    let isLast: Bool

    var body: some View {
        CardRow(isLast: isLast) {
            HStack(spacing: 12) {
                Checkbox(isOn: $isSelected)
                VStack(alignment: .leading, spacing: 3) {
                    Text(file.path.abbreviatingHome).textStyle(.body).lineLimit(1).truncationMode(.middle)
                    HStack(spacing: 8) {
                        if file.keep {
                            Badge(text: "À garder", tone: .success)
                        }
                        if let modified = Format.relative(file.modified) {
                            Text("Modifié \(modified)").textStyle(.caption)
                        }
                        // A partial clone frees less than its size.
                        if file.reclaimable < size * 9 / 10 {
                            Text("Données partagées : libère \(Format.bytes(file.reclaimable))").textStyle(.caption)
                        }
                    }
                }
                Spacer(minLength: 12)
                HoverReveal {
                    HStack(spacing: 2) {
                        IconButton(icon: .file, help: "Ouvrir") { open() }
                        IconButton(icon: .arrowUpRight, help: "Afficher dans le Finder") { SystemTrash.reveal(file.path) }
                    }
                }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityAction(named: "Ouvrir") { open() }
        .accessibilityAction(named: "Afficher dans le Finder") { SystemTrash.reveal(file.path) }
    }

    private func open() {
        NSWorkspace.shared.open(URL(fileURLWithPath: file.path))
    }
}
