import AppKit
import LevraBindings
import SwiftUI

/// The panel under the menu bar icon: how the Mac is doing, and what is
/// worth doing from there. Actions are confirmed inside the panel, which
/// closes as soon as another window takes the focus.
struct MenuBarView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.openSettings) private var openSettings
    @State private var confirming: Confirmation?
    @State private var trashRefusal: String?

    private enum Confirmation {
        case clean, emptyTrash
    }

    static let width: CGFloat = 340
    static let padding: CGFloat = 12
    static let spacing: CGFloat = 10

    var body: some View {
        VStack(spacing: Self.spacing) {
            header
            if let disk = model.disk {
                diskCard(disk)
            }
            HStack(alignment: .top, spacing: Self.spacing) {
                memoryTile
                trashTile
            }
            .fixedSize(horizontal: false, vertical: true)
            topApps
            footer
        }
        .padding(Self.padding)
        .frame(width: Self.width)
        .background(Theme.background)
        .animation(Motion.snappy, value: confirming)
        .task {
            model.refreshDisk()
            model.refreshTrash()
            await model.memory.refresh()
        }
    }

    private var header: some View {
        HStack(spacing: 10) {
            LevraMark(size: 30)
            VStack(alignment: .leading, spacing: 2) {
                Text("Levra").textStyle(.heading)
                HStack(spacing: 5) {
                    Circle().fill(status.tone.color).frame(width: 6, height: 6)
                    Text(status.text).textStyle(.caption)
                }
            }
            Spacer()
            IconButton(icon: .settings, help: "Réglages") {
                NSApplication.shared.activate()
                openSettings()
            }
        }
        .padding(.horizontal, 4)
    }

    /// The most pressing thing about the Mac, in a few words.
    private var status: (text: String, tone: Badge.Tone) {
        let disk = model.disk?.tone
        let pressure = model.memoryStats?.pressure
        if disk == .danger { return ("Disque presque plein", .danger) }
        if pressure == .critical { return ("Mémoire saturée", .danger) }
        if disk == .warning { return ("Disque bien rempli", .warning) }
        if pressure == .warning { return ("Mémoire sous pression", .warning) }
        return ("Tout va bien", .success)
    }

    private func diskCard(_ disk: DiskSpace) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 14) {
                DiskRing(disk: disk, diameter: 64)
                VStack(alignment: .leading, spacing: 2) {
                    Text("\(Format.bytes(UInt64(max(disk.available, 0)))) libres")
                        .font(.system(size: 20, weight: .semibold))
                        .tracking(-0.5)
                        .foregroundStyle(Theme.textPrimary)
                        .contentTransition(.numericText())
                    Text("sur \(Format.bytes(UInt64(max(disk.total, 0))))").textStyle(.secondary)
                }
                Spacer(minLength: 0)
            }
            Rectangle().fill(Theme.border).frame(height: 1)
            cleanupRow
        }
        .padding(14)
        .card()
    }

    @ViewBuilder
    private var cleanupRow: some View {
        let cleanup = model.cleanup
        let recommended = Format.bytes(cleanup.recommendedBytes)
        switch cleanup.phase {
        case .idle:
            ActionRow(title: "Nettoyage", detail: "Trouve ce qui peut partir sans risque.") {
                Button("Analyser") { Task { await cleanup.scan() } }
                    .buttonStyle(.app(.primary, size: .small))
            }
        case let .scanning(_, _, done):
            let finished = done.nonzeroBitCount
            ActionRow(
                title: "Analyse…",
                detail: "\(finished) \(Format.agree(finished, "vérification terminée", "vérifications terminées")) sur \(CleanupModel.steps.count)."
            ) {
                Spinner(size: 14)
            }
        case .cleaning:
            ActionRow(title: "Nettoyage…", detail: "Les caches et fichiers recréables partent.") {
                Spinner(size: 14)
            }
        case .ready where confirming == .clean:
            ActionRow(title: "Libérer \(recommended) ?", detail: "Caches et fichiers qui se recréent tout seuls.") {
                Button("Annuler") { confirming = nil }
                    .buttonStyle(.app(.ghost, size: .small))
                Button("Libérer") {
                    confirming = nil
                    Task { await model.quickClean() }
                }
                .buttonStyle(.app(.primary, size: .small))
            }
        case .ready:
            if cleanup.recommendedBytes > 0 {
                ActionRow(title: "\(recommended) à libérer", detail: "Sans risque : caches, fichiers qui se recréent.") {
                    Button("Détail") { show(.cleanup) }
                        .buttonStyle(.app(.ghost, size: .small))
                    Button("Libérer…") { confirming = .clean }
                        .buttonStyle(.app(.primary, size: .small))
                }
            } else if let outcome = cleanup.outcome {
                let failed = outcome.failures.count
                ActionRow(
                    title: "\(Format.bytes(outcome.freedBytes)) libérés",
                    detail: failed == 0
                        ? "Rien d'autre à libérer sans risque."
                        : "\(Format.counted(failed, "élément")) \(Format.agree(failed, "n'a pas pu partir", "n'ont pas pu partir"))."
                ) {
                    Button("Détail") { show(.cleanup) }
                        .buttonStyle(.app(.ghost, size: .small))
                }
            } else {
                ActionRow(title: "Rien à libérer sans risque", detail: "Le reste se trie dans Nettoyage.") {
                    Button("Voir") { show(.cleanup) }
                        .buttonStyle(.app(.ghost, size: .small))
                }
            }
        }
    }

    private var memoryTile: some View {
        let stats = model.memoryStats
        let tone = stats?.pressure.tone
        return Tile(icon: .chip, title: "Mémoire", tone: tone) {
            Text(stats.map { Format.bytes($0.used) } ?? "…").font(TileStyle.valueFont).contentTransition(.numericText())
            SizeBar(
                fraction: stats.map { Double($0.used) / Double(max($0.total, 1)) } ?? 0,
                width: TileStyle.innerWidth,
                tint: tone?.color ?? Theme.textSecondary
            )
            Text(stats.map { "sur \(Format.bytes($0.total)) · pression \($0.pressure.label.lowercased())" } ?? " ")
                .textStyle(.caption)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
            Button("Voir") { show(.memory) }
                .buttonStyle(.app(.secondary, size: .small))
        }
    }

    private var trashTile: some View {
        Tile(icon: .delete, title: "Corbeille") {
            Text(model.trashBytes.map(Format.bytes) ?? "—").font(TileStyle.valueFont).contentTransition(.numericText())
            Group {
                if let trashRefusal {
                    Text(trashRefusal).foregroundStyle(Theme.danger)
                } else if confirming == .emptyTrash {
                    Text("Supprimée définitivement, sans retour possible.")
                } else if model.trashBytes == nil {
                    Text("Taille visible avec l'accès complet au disque.")
                } else {
                    Text("libérés en la vidant.")
                }
            }
            .textStyle(.caption)
            .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
            trashAction
        }
    }

    @ViewBuilder
    private var trashAction: some View {
        if model.emptyingTrash {
            Spinner(size: 14).frame(height: 24)
        } else if confirming == .emptyTrash {
            HStack(spacing: 6) {
                Button("Annuler") { confirming = nil }
                    .buttonStyle(.app(.ghost, size: .small))
                Button("Vider") {
                    confirming = nil
                    Task { trashRefusal = await model.emptyTrash() }
                }
                .buttonStyle(.app(.danger, size: .small))
            }
        } else {
            Button("Vider…") {
                trashRefusal = nil
                confirming = .emptyTrash
            }
            .buttonStyle(.app(.secondary, size: .small))
            .disabled(model.trashBytes == 0)
        }
    }

    /// The apps using the most memory, as in the Memory page, each with a
    /// way to quit it under the pointer.
    @ViewBuilder
    private var topApps: some View {
        let apps = (model.memory.snapshot?.groups ?? []).filter { $0.kind == .app }.prefix(3)
        if !apps.isEmpty {
            VStack(alignment: .leading, spacing: 6) {
                Text("Apps qui occupent le plus de mémoire")
                    .textStyle(.label)
                    .padding(.horizontal, 4)
                VStack(spacing: 0) {
                    ForEach(Array(apps.enumerated()), id: \.element.path) { index, group in
                        CardRow(isLast: index == apps.count - 1) {
                            HStack(spacing: 10) {
                                Button { show(.memory) } label: {
                                    HStack(spacing: 10) {
                                        AppIcon(path: group.path, size: 20)
                                        Text(group.name).textStyle(.body).lineLimit(1)
                                    }
                                    .contentShape(Rectangle())
                                }
                                .buttonStyle(.plain)
                                Spacer(minLength: 8)
                                if group.quittable {
                                    HoverReveal {
                                        Button("Quitter") { Task { await model.memory.quit(group, force: false) } }
                                            .buttonStyle(.app(.secondary, size: .small))
                                    }
                                }
                                Text(Format.bytes(group.footprint))
                                    .textStyle(.secondary)
                                    .monospacedDigit()
                                    .contentTransition(.numericText())
                            }
                        }
                    }
                }
                .clipShape(RoundedRectangle(cornerRadius: Theme.radius))
                .card()
                if let quitError = model.memory.quitError {
                    Text(quitError)
                        .textStyle(.caption)
                        .foregroundStyle(Theme.warning)
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(.horizontal, 4)
                }
            }
        }
    }

    private var footer: some View {
        HStack {
            Button("Ouvrir Levra") { show(nil) }
                .buttonStyle(.app(.secondary, size: .small))
            Button("Mises à jour…") { model.updates.check() }
                .buttonStyle(.app(.ghost, size: .small))
                .disabled(!model.updates.canCheck)
            Spacer()
            Button("Quitter") { NSApplication.shared.terminate(nil) }
                .buttonStyle(.app(.ghost, size: .small))
                .keyboardShortcut("q")
        }
        .padding(.horizontal, 2)
    }

    /// Opens the window, on a page if one is given. The panel is closed
    /// first, since a window of the same app taking the focus leaves it
    /// open: with the Escape key's command, which SwiftUI follows, where
    /// closing its window behind its back costs the next click on the icon.
    private func show(_ section: AppModel.Section?) {
        NSApplication.shared.sendAction(#selector(NSResponder.cancelOperation(_:)), to: nil, from: nil)
        model.showWindow(section)
    }
}

/// A title, a line of detail and the actions that go with them.
private struct ActionRow<Actions: View>: View {
    let title: String
    let detail: String
    @ViewBuilder var actions: Actions

    var body: some View {
        HStack(spacing: 8) {
            VStack(alignment: .leading, spacing: 2) {
                Text(title).textStyle(.bodyStrong).contentTransition(.numericText())
                Text(detail).textStyle(.caption).fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 8)
            actions
        }
        .frame(minHeight: 32)
    }
}

/// Measures shared by the tiles, which a generic type cannot hold itself.
@MainActor
private enum TileStyle {
    static let padding: CGFloat = 12
    static let innerWidth = (MenuBarView.width - 2 * MenuBarView.padding - MenuBarView.spacing) / 2 - 2 * padding
    static let valueFont = Font.system(size: 18, weight: .semibold).monospacedDigit()
}

/// Half the panel's width: a figure and what to do about it.
private struct Tile<Content: View>: View {
    let icon: Papicon
    let title: String
    var tone: Badge.Tone?
    @ViewBuilder var content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 6) {
                Icon(icon, size: 14).foregroundStyle(Theme.textSecondary)
                Text(title).textStyle(.label)
                Spacer()
                if let tone {
                    Circle().fill(tone.color).frame(width: 6, height: 6)
                }
            }
            content
        }
        .foregroundStyle(Theme.textPrimary)
        .padding(TileStyle.padding)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .card()
    }
}
