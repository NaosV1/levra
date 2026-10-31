import DebarrasBindings
import SwiftUI

/// Biggest individual files of the explored folder, optionally only the
/// ones nobody touched for a while.
struct LargeFilesView: View {
    let explorer: ExplorerModel
    @Environment(AppModel.self) private var app
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var minimum: UInt64 = 500_000_000
    @State private var untouchedDays: UInt32 = 0
    @State private var files: [NodeInfo] = []
    @State private var selection: Set<UInt64> = []
    @State private var confirming = false
    @State private var outcome: CleanupOutcome?

    private struct Filter: Equatable {
        let minimum: UInt64
        let untouchedDays: UInt32
        let phase: ExplorerModel.Phase
    }

    var body: some View {
        PageScroll {
            PageHeader(title: "Gros fichiers", subtitle: subtitle) {
                if explorer.phase == .ready {
                    SegmentedControl(
                        options: [(UInt64(100_000_000), "100 Mo"), (500_000_000, "500 Mo"), (1_000_000_000, "1 Go"), (5_000_000_000, "5 Go")],
                        selection: $minimum
                    )
                    SegmentedControl(
                        options: [(UInt32(0), "Tous"), (90, "+ 3 mois"), (365, "+ 1 an")],
                        selection: $untouchedDays
                    )
                }
            }
        } content: {
            if explorer.phase == .ready {
                list
            } else if case let .scanning(files, bytes) = explorer.phase {
                ScanningState(title: "Mesure du dossier personnel…", files: files, bytes: bytes) { explorer.cancelScan() }
            } else {
                EmptyState(
                    icon: .file,
                    title: "Les plus gros fichiers",
                    message: "Utilise la mesure de l'explorateur : lance-la sur ton dossier personnel."
                ) {
                    Button("Analyser mon dossier personnel") {
                        Task { await explorer.choose(URL(fileURLWithPath: NSHomeDirectory())) }
                    }
                    .buttonStyle(.app(.primary))
                }
            }
        }
        .safeAreaInset(edge: .bottom) {
            if !selection.isEmpty {
                ActionBar {
                    Text("\(selection.count) fichier(s)").textStyle(.secondary)
                    Text(Format.bytes(selectedBytes)).textStyle(.bodyStrong).monospacedDigit().contentTransition(.numericText())
                    Spacer()
                    Button("Tout désélectionner") { withAnimation(Motion.quick) { selection = [] } }
                        .buttonStyle(.app(.ghost))
                    Button { confirming = true } label: { ButtonLabel("Mettre à la corbeille", icon: .delete) }
                        .buttonStyle(.app(.danger))
                }
                .transition(.move(edge: .bottom).combined(with: .opacity))
            }
        }
        .animation(Motion.snappy, value: selection.isEmpty)
        .animation(Motion.snappy, value: explorer.phase)
        .task(id: Filter(minimum: minimum, untouchedDays: untouchedDays, phase: explorer.phase)) { reload() }
        .confirmationDialog(
            "Mettre \(selection.count) fichier(s) à la corbeille ?",
            isPresented: $confirming,
            titleVisibility: .visible
        ) {
            Button("Mettre à la corbeille", role: .destructive) {
                let nodes = files.filter { selection.contains($0.id) }
                Task {
                    let result = await explorer.trash(nodes)
                    withAnimation(Motion.snappy) {
                        outcome = result
                        selection = []
                        reload()
                    }
                    app.refreshDisk()
                }
            }
        } message: {
            Text("\(Format.bytes(selectedBytes)). Récupérable tant que la corbeille n'est pas vidée.")
        }
    }

    private var subtitle: String {
        guard explorer.phase == .ready else { return "Repère les fichiers lourds oubliés." }
        return "\(files.count) fichier(s) · \(Format.bytes(files.reduce(0) { $0 + $1.bytes }))"
    }

    private var selectedBytes: UInt64 {
        files.filter { selection.contains($0.id) }.reduce(0) { $0 + $1.bytes }
    }

    private var list: some View {
        VStack(alignment: .leading, spacing: 16) {
            if let outcome {
                OutcomeBanner(outcome: outcome) { withAnimation(Motion.snappy) { self.outcome = nil } }
                    .transition(.lift(reduceMotion: reduceMotion))
            }
            if files.isEmpty {
                EmptyState(icon: .checkCircle, title: "Aucun fichier", message: "Rien ne correspond à ces filtres.") {}
            } else {
                VStack(spacing: 0) {
                    ForEach(Array(files.enumerated()), id: \.element.id) { index, file in
                        FileRow(
                            file: file,
                            path: explorer.path(of: file),
                            isSelected: binding(for: file),
                            isLast: index == files.count - 1
                        )
                        .staggeredAppear(index: index)
                    }
                }
                .card()
            }
        }
    }

    private func binding(for file: NodeInfo) -> Binding<Bool> {
        Binding(
            get: { selection.contains(file.id) },
            set: { if $0 { selection.insert(file.id) } else { selection.remove(file.id) } }
        )
    }

    private func reload() {
        files = explorer.largestFiles(minBytes: minimum, untouchedDays: untouchedDays)
        selection.formIntersection(files.map(\.id))
    }
}

private struct FileRow: View {
    let file: NodeInfo
    let path: String?
    @Binding var isSelected: Bool
    let isLast: Bool

    var body: some View {
        CardRow(isLast: isLast) {
            HStack(spacing: 12) {
                Checkbox(isOn: $isSelected)
                if let path {
                    AppIcon(path: path, size: 20)
                }
                VStack(alignment: .leading, spacing: 2) {
                    Text(file.name).textStyle(.body).lineLimit(1)
                    Text(path?.abbreviatingHome ?? "").textStyle(.mono).lineLimit(1).truncationMode(.middle)
                }
                Spacer(minLength: 12)
                if let path {
                    HoverReveal {
                        IconButton(icon: .arrowUpRight, help: "Afficher dans le Finder") { SystemTrash.reveal(path) }
                    }
                }
                if let modified = Format.relative(file.modified) {
                    Text(modified).textStyle(.caption)
                }
                Text(Format.bytes(file.bytes))
                    .textStyle(.bodyStrong)
                    .monospacedDigit()
                    .frame(width: 76, alignment: .trailing)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityAction(named: "Afficher dans le Finder") { if let path { SystemTrash.reveal(path) } }
    }
}
