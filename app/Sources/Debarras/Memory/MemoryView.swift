import AppKit
import DebarrasBindings
import SwiftUI

struct MemoryView: View {
    let model: MemoryModel
    @Environment(AppModel.self) private var app
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var pending: PendingQuit?

    /// A quit waiting for confirmation.
    private struct PendingQuit {
        let group: ProcessGroupInfo
        let force: Bool
    }

    var body: some View {
        PageScroll {
            PageHeader(title: "Mémoire", subtitle: subtitle) {
                LiveIndicator()
            }
        } content: {
            if let snapshot = model.snapshot {
                content(snapshot)
            } else if let error = model.error {
                EmptyState(icon: .chip, title: "Mémoire illisible", message: error) {}
            } else {
                HStack { Spacer(); Spinner(size: 20); Spacer() }.padding(.vertical, 72)
            }
        }
        .task { await model.runLiveUpdates() }
        .confirmationDialog(
            pending.map(confirmationTitle) ?? "",
            isPresented: Binding(get: { pending != nil }, set: { if !$0 { pending = nil } }),
            titleVisibility: .visible
        ) {
            if let pending {
                Button(pending.force ? "Forcer à quitter" : "Arrêter", role: .destructive) { quit(pending.group, force: pending.force) }
            }
        } message: {
            Text(pending?.force == true
                ? "Les modifications non enregistrées seront perdues."
                : "Le programme peut être en train de travailler pour toi (serveur, compilation…).")
        }
    }

    private var subtitle: String {
        guard let stats = model.snapshot?.stats else { return "Ce qui occupe la mémoire vive, en direct." }
        return "\(Format.bytes(stats.used)) utilisés sur \(Format.bytes(stats.total)) · pression \(stats.pressure.label.lowercased())"
    }

    private func content(_ snapshot: MemorySnapshotInfo) -> some View {
        VStack(alignment: .leading, spacing: 20) {
            MemoryOverview(stats: snapshot.stats)
            if let quitError = model.quitError {
                Banner(tone: .warning, icon: .alert, title: quitError) {
                    IconButton(icon: .cancel, help: "Fermer") { withAnimation(Motion.snappy) { model.dismissQuitError() } }
                }
                .transition(.lift(reduceMotion: reduceMotion))
            }
            VStack(alignment: .leading, spacing: 8) {
                SectionHeader(title: "Ce qui utilise la mémoire", icon: .chip)
                let groups = Array(snapshot.groups.prefix(60))
                VStack(spacing: 0) {
                    ForEach(Array(groups.enumerated()), id: \.element.path) { index, group in
                        GroupRow(group: group, total: snapshot.stats.total, isLast: index == groups.count - 1) { force in
                            if group.kind == .app && !force {
                                quit(group, force: false)
                            } else {
                                pending = PendingQuit(group: group, force: force)
                            }
                        }
                        .staggeredAppear(index: index)
                    }
                }
                .card()
                Text("\(snapshot.unmeasured) processus d'autres utilisateurs (WindowServer, services système) ne se mesurent pas sans droits administrateur ; leur mémoire est comprise dans la mémoire utilisée.")
                    .textStyle(.caption)
                    .padding(.horizontal, 4)
            }
        }
        .animation(Motion.snappy, value: model.quitError)
    }

    private func quit(_ group: ProcessGroupInfo, force: Bool) {
        Task {
            await model.quit(group, force: force)
            app.refreshMemory()
        }
    }

    private func confirmationTitle(_ quit: PendingQuit) -> String {
        let count = quit.group.processes.count
        let what = count > 1 ? "\(quit.group.name) (\(count) processus)" : quit.group.name
        return quit.force ? "Forcer \(what) à quitter ?" : "Arrêter \(what) ?"
    }
}

/// A dot that breathes while figures refresh; still with Reduce Motion.
private struct LiveIndicator: View {
    @State private var dim = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        HStack(spacing: 6) {
            Circle()
                .fill(Theme.success)
                .frame(width: 6, height: 6)
                .opacity(dim && !reduceMotion ? 0.35 : 1)
                .animation(.easeInOut(duration: 1.2).repeatForever(autoreverses: true), value: dim)
            Text("En direct").textStyle(.label)
        }
        .padding(.horizontal, 10)
        .frame(height: 26)
        .card()
        .onAppear { dim = true }
    }
}

private struct MemoryOverview: View {
    let stats: MemoryStatsInfo

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            VStack(alignment: .leading, spacing: 14) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(Format.bytes(stats.used))
                        .textStyle(.metric)
                        .contentTransition(.numericText())
                    Text("sur \(Format.bytes(stats.total))").textStyle(.secondary)
                    Spacer()
                    Badge(text: "Pression \(stats.pressure.label.lowercased())", tone: stats.pressure.tone)
                }
                CompositionBar(segments: segments, total: stats.total)
                HStack(spacing: 16) {
                    ForEach(segments, id: \.label) { segment in
                        HStack(spacing: 6) {
                            RoundedRectangle(cornerRadius: 2).fill(segment.color).frame(width: 8, height: 8)
                            Text(segment.label).textStyle(.caption)
                        }
                    }
                }
                Text("\(stats.pressure.explanation) La mémoire libre n'est pas un objectif : macOS s'en sert comme cache et la rend dès qu'une app en a besoin. Pour en récupérer vraiment, quitte ce qui en prend le plus.")
                    .textStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .padding(18)
            .card()
            .animation(Motion.snappy, value: stats.used)

            HStack(spacing: 12) {
                StatTile(label: "Apps", value: Format.bytes(stats.app), tone: .accent)
                StatTile(label: "Verrouillée", value: Format.bytes(stats.wired), tone: .warning)
                StatTile(label: "Compressée", value: Format.bytes(stats.compressed), tone: .neutral)
                StatTile(label: "Cache fichiers", value: Format.bytes(stats.cachedFiles), detail: "Rendu à la demande")
                StatTile(label: "Swap", value: Format.bytes(stats.swapUsed), detail: "sur \(Format.bytes(stats.swapTotal))")
            }
        }
    }

    private var segments: [CompositionBar.Segment] {
        let taken = stats.used + stats.cachedFiles
        return [
            .init(label: "Apps", bytes: stats.app, color: Theme.accent),
            .init(label: "Verrouillée", bytes: stats.wired, color: Theme.warning),
            .init(label: "Compressée", bytes: stats.compressed, color: Theme.textSecondary),
            .init(label: "Cache", bytes: stats.cachedFiles, color: Theme.borderStrong),
            .init(label: "Libre", bytes: stats.total > taken ? stats.total - taken : 0, color: Theme.border),
        ]
    }
}

private struct CompositionBar: View {
    struct Segment {
        let label: String
        let bytes: UInt64
        let color: Color
    }

    let segments: [Segment]
    let total: UInt64

    var body: some View {
        GeometryReader { geometry in
            HStack(spacing: 2) {
                ForEach(segments, id: \.label) { segment in
                    RoundedRectangle(cornerRadius: 2)
                        .fill(segment.color)
                        .frame(width: total > 0 ? max(0, geometry.size.width * Double(segment.bytes) / Double(total) - 2) : 0)
                }
            }
        }
        .frame(height: 10)
        .animation(Motion.snappy, value: segments.map(\.bytes))
    }
}

private struct GroupRow: View {
    let group: ProcessGroupInfo
    let total: UInt64
    let isLast: Bool
    let onQuit: (_ force: Bool) -> Void
    @State private var expanded = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        CardRow(isLast: isLast) {
            VStack(alignment: .leading, spacing: 10) {
                HStack(spacing: 12) {
                    Button {
                        withAnimation(Motion.snappy) { expanded.toggle() }
                    } label: {
                        HStack(spacing: 10) {
                            Icon(.arrowRight, size: 12)
                                .foregroundStyle(Theme.textTertiary)
                                .rotationEffect(.degrees(expanded ? 90 : 0))
                            icon.frame(width: 20, height: 20)
                            VStack(alignment: .leading, spacing: 2) {
                                HStack(spacing: 6) {
                                    Text(group.name).textStyle(.bodyStrong).lineLimit(1)
                                    if group.processes.count > 1 {
                                        Text("\(group.processes.count)").textStyle(.caption).monospacedDigit()
                                    }
                                }
                                if let hint = group.kind.hint {
                                    Text(hint).textStyle(.caption).lineLimit(1)
                                }
                            }
                        }
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    Spacer(minLength: 12)
                    actions
                    SizeBar(fraction: total > 0 ? Double(group.footprint) / Double(total) : 0, width: 64)
                    Text(Format.bytes(group.footprint))
                        .textStyle(.bodyStrong)
                        .monospacedDigit()
                        .contentTransition(.numericText())
                        .frame(width: 76, alignment: .trailing)
                }
                if expanded {
                    VStack(spacing: 6) {
                        ForEach(group.processes, id: \.pid) { process in
                            HStack(spacing: 8) {
                                Text(process.name).textStyle(.secondary).lineLimit(1).truncationMode(.middle)
                                Text("PID \(process.pid)").textStyle(.mono)
                                Spacer()
                                Text(Format.bytes(process.footprint)).textStyle(.secondary).monospacedDigit()
                            }
                            .help(process.path)
                        }
                    }
                    .padding(.leading, 52)
                    .transition(.lift(reduceMotion: reduceMotion))
                }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityAction(named: group.kind == .app ? "Quitter" : "Arrêter") { if group.quittable { onQuit(false) } }
        .accessibilityAction(named: "Forcer à quitter") { if group.quittable { onQuit(true) } }
    }

    @ViewBuilder
    private var actions: some View {
        if group.quittable {
            HoverReveal {
                HStack(spacing: 4) {
                    Button(group.kind == .app ? "Quitter" : "Arrêter") { onQuit(false) }
                        .buttonStyle(.app(.secondary, size: .small))
                    IconButton(icon: .stop, help: "Forcer à quitter", tint: Theme.danger) { onQuit(true) }
                }
            }
        } else {
            Icon(.lock, size: 14)
                .foregroundStyle(Theme.textTertiary)
                .help("Débarras ne propose pas d'arrêter ce processus.")
        }
    }

    @ViewBuilder
    private var icon: some View {
        if group.kind == .app {
            AppIcon(path: group.path, size: 20)
        } else {
            Icon(group.kind.icon, size: 16).foregroundStyle(Theme.textSecondary)
        }
    }
}
