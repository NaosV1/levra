import LevraBindings
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
            case let .scanning(files, bytes, done):
                ScanningState(
                    title: "Analyse du Mac…",
                    icon: .clean,
                    files: files,
                    bytes: bytes,
                    steps: CleanupModel.steps.enumerated().map { index, step in
                        .init(label: step.label, done: done & (1 << index) != 0)
                    }
                ) { model.cancelScan() }
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
        case .idle: "Ce qui prend de la place pour rien, et ce qu'il devient si tu le retires."
        case .scanning: "Lecture du disque, rien n'est modifié."
        case .ready, .cleaning: "Du plus sûr au plus délicat. Rien ne part sans ta confirmation."
        }
    }

    private var start: some View {
        EmptyState(
            icon: .clean,
            title: "Qu'est-ce qui peut partir ?",
            message: "Levra cherche ce qui se recrée tout seul (caches, fichiers de développement…) et te dit ce qui peut partir sans risque."
        ) {
            Button("Analyser mon Mac") { Task { await model.scan() } }
                .buttonStyle(.app(.primary, size: .large))
            if let error = model.error {
                Text(error).textStyle(.secondary).foregroundStyle(Theme.danger)
            }
        }
    }

    private var results: some View {
        VStack(alignment: .leading, spacing: 28) {
            if let outcome = model.outcome {
                OutcomeBanner(outcome: outcome) { withAnimation(Motion.snappy) { model.dismissOutcome() } }
                    .transition(.lift(reduceMotion: reduceMotion))
            }
            if !model.denied.isEmpty {
                FullDiskAccessBanner(deniedCount: model.denied.count)
            }
            if model.findings.isEmpty {
                EmptyState(icon: .checkCircle, title: "Rien de notable", message: "Aucun élément assez gros pour valoir la peine.") {}
            } else {
                summary
            }
            let sections = model.sections
            ForEach(Array(sections.enumerated()), id: \.element.decision) { sectionIndex, section in
                // The cascade runs across sections, as one list.
                let offset = sections.prefix(sectionIndex).reduce(0) { $0 + $1.findings.count }
                VStack(alignment: .leading, spacing: 10) {
                    DecisionHeader(decision: section.decision, count: section.findings.count, bytes: section.bytes)
                    VStack(spacing: 0) {
                        ForEach(Array(section.findings.enumerated()), id: \.element.id) { index, finding in
                            FindingRow(
                                finding: finding,
                                isSelected: binding(for: finding),
                                isLast: index == section.findings.count - 1
                            )
                            .staggeredAppear(index: offset + index)
                        }
                    }
                    .card()
                }
            }
        }
        .animation(Motion.snappy, value: model.findings.map(\.id))
    }

    /// The one figure that matters, before any list.
    private var summary: some View {
        HStack(spacing: 18) {
            ZStack {
                Circle().fill(Theme.success.opacity(0.12))
                Icon(.clean, size: 26, weight: 1.2).foregroundStyle(Theme.success)
            }
            .frame(width: 60, height: 60)
            VStack(alignment: .leading, spacing: 4) {
                Text(model.recommendedBytes > 0
                    ? "\(Format.bytes(model.recommendedBytes)) à libérer sans risque"
                    : "Rien à libérer sans risque")
                    .font(.system(size: 22, weight: .semibold))
                    .tracking(-0.5)
                    .foregroundStyle(Theme.textPrimary)
                    .contentTransition(.numericText())
                Text("\(Format.bytes(totalBytes)) trouvés en tout. Le reste est à regarder avant de le retirer.")
                    .textStyle(.secondary)
            }
            Spacer(minLength: 0)
        }
        .padding(20)
        .card()
        .accessibilityElement(children: .combine)
    }

    private var totalBytes: UInt64 {
        model.findings.reduce(0) { $0 + $1.bytes }
    }

    private var footer: some View {
        ActionBar {
            Text("\(model.selectedFindings.count) \(Format.agree(model.selectedFindings.count, "sélectionné", "sélectionnés"))")
                .textStyle(.secondary)
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
            return "\(kind.label) : \(Format.counted(items.count, "élément")), \(bytes)"
        }
        .joined(separator: "\n")
    }
}

extension CleanupModel.Decision {
    var title: String {
        switch self {
        case .safe: "Sans risque"
        case .recentlyUsed: "Sans risque, mais servi récemment"
        case .review: "À vérifier"
        case .careful: "À garder sauf certitude"
        }
    }

    var explanation: String {
        switch self {
        case .safe: "Se recrée tout seul quand on en a besoin. Déjà coché pour toi."
        case .recentlyUsed: "Tes projets s'en sont servis ces 30 derniers jours : le retirer t'obligerait à attendre qu'il se recrée."
        case .review: "Récupérable, mais il faudra réinstaller, retélécharger ou rebrancher un appareil."
        case .careful: "Peut contenir des données impossibles à recréer."
        }
    }

    var tone: Badge.Tone {
        switch self {
        case .safe: .success
        case .recentlyUsed: .accent
        case .review: .warning
        case .careful: .danger
        }
    }
}

private struct DecisionHeader: View {
    let decision: CleanupModel.Decision
    let count: Int
    let bytes: UInt64

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 8) {
                Circle().fill(decision.tone.color).frame(width: 8, height: 8)
                Text(decision.title).textStyle(.heading)
                Text("\(count)").textStyle(.caption).monospacedDigit()
                Spacer()
                Text(Format.bytes(bytes)).textStyle(.bodyStrong).monospacedDigit()
            }
            Text(decision.explanation)
                .textStyle(.secondary)
                .padding(.leading, 16)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.horizontal, 4)
        .accessibilityElement(children: .combine)
    }
}

/// A finding in a sentence; how it is removed, where it lives and when it
/// was last used open on demand.
private struct FindingRow: View {
    let finding: FindingInfo
    @Binding var isSelected: Bool
    let isLast: Bool
    @State private var expanded = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        CardRow(isLast: isLast) {
            VStack(alignment: .leading, spacing: 12) {
                HStack(alignment: .top, spacing: 12) {
                    Checkbox(isOn: $isSelected)
                        .disabled(finding.action == .manual)
                        .padding(.top, 1)
                    Icon(finding.category.icon, size: 16)
                        .foregroundStyle(Theme.textSecondary)
                        .padding(.top, 1)
                        .help(finding.category.label)
                        .accessibilityHidden(false)
                        .accessibilityLabel(finding.category.label)
                    VStack(alignment: .leading, spacing: 3) {
                        HStack(spacing: 8) {
                            Text(finding.title).textStyle(.bodyStrong).lineLimit(1).truncationMode(.middle)
                            if finding.action == .manual {
                                Badge(text: "À faire toi-même", tone: .neutral)
                            }
                        }
                        Text(finding.detail)
                            .textStyle(.secondary)
                            .lineLimit(expanded ? nil : 2)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    Spacer(minLength: 12)
                    Text(Format.bytes(finding.bytes)).textStyle(.bodyStrong).monospacedDigit()
                    Icon(.arrowDown, size: 12)
                        .foregroundStyle(Theme.textTertiary)
                        .rotationEffect(.degrees(expanded ? 180 : 0))
                        .padding(.top, 3)
                }
                .contentShape(Rectangle())
                .onTapGesture { withAnimation(Motion.snappy) { expanded.toggle() } }

                if expanded {
                    details
                        .padding(.leading, 56)
                        .transition(.lift(reduceMotion: reduceMotion))
                }
            }
        }
        .contextMenu {
            if let path = finding.paths.first {
                Button("Afficher dans le Finder") { SystemTrash.reveal(path) }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityAction(named: expanded ? "Masquer les détails" : "Afficher les détails") {
            expanded.toggle()
        }
        .accessibilityAction(named: "Afficher dans le Finder") {
            if let path = finding.paths.first { SystemTrash.reveal(path) }
        }
    }

    private var details: some View {
        VStack(alignment: .leading, spacing: 8) {
            Label { Text(finding.action.label) } icon: { Icon(.info, size: 12) }
            if let command = finding.command {
                HStack(spacing: 5) {
                    Icon(.commandLine, size: 12)
                    Text(command).textStyle(.mono).lineLimit(1).truncationMode(.middle)
                }
                .padding(.horizontal, 6)
                .frame(height: 20)
                .background(Theme.surfaceHover, in: RoundedRectangle(cornerRadius: 4))
            }
            if let path = finding.paths.first {
                HStack(spacing: 6) {
                    Text(path.abbreviatingHome).textStyle(.mono).lineLimit(1).truncationMode(.middle)
                    IconButton(icon: .arrowUpRight, help: "Afficher dans le Finder") { SystemTrash.reveal(path) }
                }
            }
            if let used = Format.relative(finding.lastUsed) {
                Label { Text("Dernière activité \(used)") } icon: { Icon(.clock, size: 12) }
            }
        }
        .textStyle(.caption)
    }
}
