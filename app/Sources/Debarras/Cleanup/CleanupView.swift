import DebarrasBindings
import SwiftUI

struct CleanupView: View {
    @Bindable var model: CleanupModel
    @Environment(AppModel.self) private var app
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var confirming = false

    var body: some View {
        PageScroll {
            PageHeader(title: "Nettoyage", subtitle: subtitle) {
                Button {
                    Task { await model.scan() }
                } label: {
                    ButtonLabel(model.phase == .idle ? "Analyser" : "Relancer", icon: .refresh)
                }
                .buttonStyle(.app(.secondary))
                .keyboardShortcut("r", modifiers: .command)
                .disabled(isBusy)
            }
        } content: {
            switch model.phase {
            case .idle:
                start
            case let .scanning(files, bytes):
                ScanningState(title: "Analyse du Mac…", files: files, bytes: bytes) { model.cancelScan() }
            case .ready, .cleaning:
                results
            }
        }
        .safeAreaInset(edge: .bottom) {
            if case .ready = model.phase, !model.findings.isEmpty {
                footer.transition(.move(edge: .bottom).combined(with: .opacity))
            }
        }
        .animation(Motion.snappy, value: model.phase)
        .confirmationDialog(
            "Libérer \(Format.bytes(model.selectedBytes)) ?",
            isPresented: $confirming,
            titleVisibility: .visible
        ) {
            Button("Libérer", role: .destructive) {
                Task {
                    await model.clean()
                    app.refreshDisk()
                }
            }
        } message: {
            Text(confirmationMessage)
        }
    }

    private var isBusy: Bool {
        switch model.phase {
        case .scanning, .cleaning: true
        case .idle, .ready: false
        }
    }

    private var subtitle: String {
        switch model.phase {
        case .idle: "Builds, caches, simulateurs, Docker… avec ce que chaque élément devient."
        case .scanning: "Lecture du disque, rien n'est modifié."
        case .ready, .cleaning:
            "\(model.findings.count) éléments · \(Format.bytes(model.recommendedBytes)) recommandés"
        }
    }

    private var start: some View {
        EmptyState(
            icon: .clean,
            title: "Qu'est-ce qui peut partir ?",
            message: "Chaque élément indique sa taille réelle, s'il se régénère et l'action exacte qui sera faite. Rien n'est supprimé sans ta confirmation."
        ) {
            Button("Analyser mon Mac") { Task { await model.scan() } }
                .buttonStyle(.app(.primary))
            if let error = model.error {
                Text(error).textStyle(.secondary).foregroundStyle(Theme.danger)
            }
        }
    }

    private var results: some View {
        VStack(alignment: .leading, spacing: 20) {
            if let outcome = model.outcome {
                OutcomeBanner(outcome: outcome) { withAnimation(Motion.snappy) { model.dismissOutcome() } }
                    .transition(.lift(reduceMotion: reduceMotion))
            }
            if !model.denied.isEmpty {
                FullDiskAccessBanner(deniedCount: model.denied.count)
            }
            HStack(spacing: 12) {
                StatTile(label: "Recommandé", value: Format.bytes(model.recommendedBytes), detail: "Sûr et inutilisé", tone: .success)
                StatTile(label: "Sélectionné", value: Format.bytes(model.selectedBytes), detail: "\(model.selectedFindings.count) élément(s)")
                StatTile(label: "Au total", value: Format.bytes(totalBytes), detail: "\(model.findings.count) éléments trouvés")
            }
            if model.findings.isEmpty {
                EmptyState(icon: .checkCircle, title: "Rien de notable", message: "Aucun élément assez gros pour valoir la peine.") {}
            }
            ForEach(Array(model.groups.enumerated()), id: \.element.category) { groupIndex, group in
                VStack(alignment: .leading, spacing: 8) {
                    SectionHeader(title: group.category.label, icon: group.category.icon, trailing: Format.bytes(group.bytes))
                    VStack(spacing: 0) {
                        ForEach(Array(group.findings.enumerated()), id: \.element.id) { index, finding in
                            FindingRow(
                                finding: finding,
                                isSelected: binding(for: finding),
                                isLast: index == group.findings.count - 1
                            )
                            .staggeredAppear(index: groupIndex * 3 + index)
                        }
                    }
                    .card()
                }
            }
        }
        .animation(Motion.snappy, value: model.findings.map(\.id))
    }

    private var totalBytes: UInt64 {
        model.findings.reduce(0) { $0 + $1.bytes }
    }

    private var footer: some View {
        ActionBar {
            Text("\(model.selectedFindings.count) sélectionné(s)").textStyle(.secondary)
            Text(Format.bytes(model.selectedBytes))
                .textStyle(.bodyStrong)
                .monospacedDigit()
                .contentTransition(.numericText())
            Spacer()
            Button("Sélection recommandée") {
                withAnimation(Motion.quick) {
                    model.selection = Set(model.findings.filter(\.recommended).map(\.id))
                }
            }
            .buttonStyle(.app(.ghost))
            Button {
                confirming = true
            } label: {
                ButtonLabel("Libérer \(Format.bytes(model.selectedBytes))", icon: .clean)
            }
            .buttonStyle(.app(.primary))
            .disabled(model.selection.isEmpty || isBusy)
        }
        .animation(Motion.snappy, value: model.selectedBytes)
    }

    private func binding(for finding: FindingInfo) -> Binding<Bool> {
        Binding(
            get: { model.selection.contains(finding.id) },
            set: { selected in
                if selected { model.selection.insert(finding.id) } else { model.selection.remove(finding.id) }
            }
        )
    }

    private var confirmationMessage: String {
        let byKind = Dictionary(grouping: model.selectedFindings, by: \.action)
        return [ActionKind.delete, .trash, .command].compactMap { kind -> String? in
            guard let items = byKind[kind], !items.isEmpty else { return nil }
            let bytes = Format.bytes(items.reduce(0) { $0 + $1.bytes })
            return "\(kind.label) : \(items.count) élément(s), \(bytes)"
        }
        .joined(separator: "\n")
    }
}

private struct FindingRow: View {
    let finding: FindingInfo
    @Binding var isSelected: Bool
    let isLast: Bool

    var body: some View {
        CardRow(isLast: isLast) {
            HStack(alignment: .top, spacing: 12) {
                Checkbox(isOn: $isSelected)
                    .disabled(finding.action == .manual)
                    .padding(.top, 1)
                VStack(alignment: .leading, spacing: 4) {
                    HStack(spacing: 8) {
                        Text(finding.title).textStyle(.bodyStrong).lineLimit(1)
                        Badge(text: finding.safety.label, tone: finding.safety.tone)
                    }
                    Text(finding.detail)
                        .textStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                    meta
                }
                Spacer(minLength: 12)
                VStack(alignment: .trailing, spacing: 6) {
                    Text(Format.bytes(finding.bytes)).textStyle(.bodyStrong).monospacedDigit()
                    if let path = finding.paths.first {
                        HoverReveal {
                            IconButton(icon: .arrowUpRight, help: "Afficher dans le Finder") { SystemTrash.reveal(path) }
                        }
                    }
                }
            }
        }
        .contextMenu {
            if let path = finding.paths.first {
                Button("Afficher dans le Finder") { SystemTrash.reveal(path) }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityAction(named: "Afficher dans le Finder") {
            if let path = finding.paths.first { SystemTrash.reveal(path) }
        }
    }

    @ViewBuilder
    private var meta: some View {
        let used = Format.relative(finding.lastUsed)
        if finding.command != nil || finding.action == .manual || used != nil {
            HStack(spacing: 12) {
                if let command = finding.command {
                    HStack(spacing: 5) {
                        Icon(.commandLine, size: 12)
                        Text(command).textStyle(.mono)
                    }
                    .padding(.horizontal, 6)
                    .frame(height: 20)
                    .background(Theme.surfaceHover, in: RoundedRectangle(cornerRadius: 4))
                } else if finding.action == .manual {
                    Label { Text(finding.action.label) } icon: { Icon(.info, size: 12) }
                }
                if let used {
                    Label { Text("Activité \(used)") } icon: { Icon(.clock, size: 12) }
                }
            }
            .textStyle(.caption)
            .padding(.top, 2)
        }
    }
}
