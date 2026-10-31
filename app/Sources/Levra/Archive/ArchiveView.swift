import AppKit
import LevraBindings
import SwiftUI

struct ArchiveView: View {
    @Bindable var model: ArchiveModel
    @Environment(AppModel.self) private var app
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var confirming = false

    var body: some View {
        PageScroll {
            PageHeader(title: "Vieux projets", subtitle: subtitle) {
                SegmentedControl(
                    options: [(3, "3 mois"), (6, "6 mois"), (12, "1 an"), (24, "2 ans")],
                    selection: $model.idleMonths
                )
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
                    icon: .package,
                    title: "Des projets qui dorment ?",
                    message: "Levra repère les projets de développement que tu n'as pas touchés depuis longtemps. Chacun peut être compressé sans ses dépendances (elles reviennent au prochain install), rangé où tu veux, puis l'original part à la corbeille."
                ) {
                    Button("Rechercher les vieux projets") { Task { await model.find() } }
                        .buttonStyle(.app(.primary, size: .large))
                    if let error = model.error {
                        Text(error).textStyle(.secondary).foregroundStyle(Theme.danger)
                    }
                }
            case let .scanning(files, bytes):
                ScanningState(title: "Recherche des projets…", icon: .package, files: files, bytes: bytes) { model.cancelScan() }
            case .ready, .archiving:
                results
            }
        }
        .safeAreaInset(edge: .bottom) {
            switch model.phase {
            case .ready where !model.projects.isEmpty:
                footer.transition(.move(edge: .bottom).combined(with: .opacity))
            case .archiving:
                progressBar.transition(.move(edge: .bottom).combined(with: .opacity))
            default:
                EmptyView()
            }
        }
        .animation(Motion.snappy, value: model.phase)
        .confirmationDialog(
            "Archiver \(Format.counted(model.selection.count, "projet")) ?",
            isPresented: $confirming,
            titleVisibility: .visible
        ) {
            Button("Archiver") {
                Task {
                    await model.archiveSelected()
                    app.refreshDisk()
                }
            }
        } message: {
            Text(confirmationMessage)
        }
    }

    private var subtitle: String {
        switch model.phase {
        case .idle: "Les projets oubliés, compressés ou rangés sur un autre disque."
        case .scanning: "Lecture des dossiers de projets, rien n'est modifié."
        case .archiving: "Archivage en cours, un projet après l'autre."
        case .ready:
            "\(Format.counted(model.projects.count, "projet")) · \(Format.bytes(model.projects.reduce(0) { $0 + $1.bytes }))"
        }
    }

    private var confirmationMessage: String {
        let uncommitted = model.selectedProjects.filter(\.hasUncommitted).count
        var lines = ["Chaque projet est compressé dans \(model.destination.path.abbreviatingHome), l'archive est relue, puis l'original va à la corbeille."]
        if uncommitted > 0 {
            lines.append("\(Format.counted(uncommitted, "projet")) \(Format.agree(uncommitted, "a", "ont")) des changements pas commités : ils sont gardés dans l'archive.")
        }
        return lines.joined(separator: "\n\n")
    }

    private var results: some View {
        VStack(alignment: .leading, spacing: 16) {
            if let outcome = model.outcome {
                OutcomeBanner(outcome: outcome) { withAnimation(Motion.snappy) { model.dismissOutcome() } }
                    .transition(.lift(reduceMotion: reduceMotion))
                if model.archivesCreated > 0 {
                    Banner(
                        tone: .neutral,
                        icon: .package,
                        title: "\(Format.counted(model.archivesCreated, "archive")) · \(Format.bytes(model.archivesBytes))",
                        message: "Rangées dans \(model.destination.path.abbreviatingHome). Double-clic sur une archive pour retrouver le projet."
                    ) {
                        Button("Afficher") { NSWorkspace.shared.open(model.destination) }
                            .buttonStyle(.app(.secondary, size: .small))
                    }
                }
            }
            destinationCard
            if model.projects.isEmpty {
                EmptyState(icon: .checkCircle, title: "Aucun vieux projet", message: "Tous tes projets ont bougé ces \(model.idleMonths) derniers mois.") {}
            } else {
                VStack(spacing: 0) {
                    ForEach(Array(model.projects.enumerated()), id: \.element.path) { index, project in
                        ProjectRow(
                            project: project,
                            isSelected: Binding(
                                get: { model.selection.contains(project.path) },
                                set: { if $0 { model.selection.insert(project.path) } else { model.selection.remove(project.path) } }
                            ),
                            isLast: index == model.projects.count - 1,
                            activity: activity(of: project)
                        )
                        .staggeredAppear(index: index)
                        .transition(.dismissRow(reduceMotion: reduceMotion))
                    }
                }
                .card()
            }
        }
        .animation(Motion.snappy, value: model.projects.map(\.path))
    }

    private var destinationCard: some View {
        HStack(spacing: 12) {
            Icon(.hardDrive, size: 16).foregroundStyle(Theme.textSecondary)
            VStack(alignment: .leading, spacing: 2) {
                Text("Archives rangées dans \(model.destination.path.abbreviatingHome)").textStyle(.bodyStrong)
                Text(model.destinationIsOtherDisk
                    ? "Sur un autre disque : toute la place des projets est libérée ici."
                    : "Sur ce Mac : choisis un disque externe pour libérer toute la place.")
                    .textStyle(.caption)
            }
            Spacer()
            Button("Changer…") { chooseDestination() }
                .buttonStyle(.app(.secondary, size: .small))
                .disabled(model.phase == .archiving)
        }
        .padding(14)
        .card()
    }

    private var footer: some View {
        ActionBar {
            Text(Format.counted(model.selection.count, "projet")).textStyle(.secondary)
            Text("≈ \(Format.bytes(model.estimatedFreed)) libérés").textStyle(.bodyStrong).monospacedDigit().contentTransition(.numericText())
            Spacer()
            Button(model.selection.count == model.projects.count ? "Tout désélectionner" : "Tout sélectionner") {
                withAnimation(Motion.quick) {
                    model.selection = model.selection.count == model.projects.count ? [] : Set(model.projects.map(\.path))
                }
            }
            .buttonStyle(.app(.ghost))
            Button { confirming = true } label: {
                ButtonLabel("Archiver \(Format.counted(model.selection.count, "projet"))", icon: .package)
            }
            .buttonStyle(.app(.primary))
            .disabled(model.selection.isEmpty)
        }
        .animation(Motion.snappy, value: model.estimatedFreed)
    }

    private var progressBar: some View {
        ActionBar {
            Spinner()
            VStack(alignment: .leading, spacing: 5) {
                HStack(spacing: 6) {
                    Text("Archivage en cours").textStyle(.bodyStrong)
                    Text("\(min(model.done + 1, model.total)) sur \(model.total)")
                        .textStyle(.secondary)
                        .monospacedDigit()
                        .contentTransition(.numericText())
                }
                SizeBar(
                    fraction: model.total > 0 ? (Double(model.done) + (model.current == nil ? 0 : 0.35)) / Double(model.total) : 0,
                    width: 220,
                    tint: Theme.accent
                )
            }
            Spacer()
            Button("Arrêter") { withAnimation(Motion.snappy) { model.stop() } }
                .buttonStyle(.app(.secondary))
                .disabled(model.queue.isEmpty)
        }
        .animation(Motion.snappy, value: model.done)
    }

    private func activity(of project: OldProjectInfo) -> ProjectRow.Activity {
        if model.current == project.path { return .working }
        if model.queue.contains(project.path) { return .queued }
        return model.phase == .archiving ? .locked : .none
    }

    private func chooseDestination() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.canCreateDirectories = true
        panel.directoryURL = model.destination
        panel.prompt = "Ranger ici"
        panel.message = "Où ranger les archives des projets ? Un disque externe libère toute la place."
        guard panel.runModal() == .OK, let url = panel.url else { return }
        model.destination = url
    }
}

private struct ProjectRow: View {
    enum Activity {
        case none, queued, working, locked
    }

    let project: OldProjectInfo
    @Binding var isSelected: Bool
    let isLast: Bool
    let activity: Activity

    var body: some View {
        CardRow(isLast: isLast) {
            HStack(spacing: 12) {
                ZStack {
                    if activity == .working {
                        Spinner().transition(.scale(scale: 0.6).combined(with: .opacity))
                    } else {
                        Checkbox(isOn: $isSelected)
                            .disabled(activity != .none)
                            .transition(.scale(scale: 0.6).combined(with: .opacity))
                    }
                }
                .frame(width: 16, height: 16)
                Icon(.sourceCode, size: 16).foregroundStyle(Theme.textSecondary)
                VStack(alignment: .leading, spacing: 3) {
                    HStack(spacing: 8) {
                        Text(project.name).textStyle(.bodyStrong).lineLimit(1)
                        if project.isGit {
                            Badge(text: "Git", tone: .neutral)
                        }
                        if project.hasUncommitted {
                            Badge(text: "Changements pas commités", tone: .warning)
                        }
                    }
                    HStack(spacing: 6) {
                        Text(project.path.abbreviatingHome).textStyle(.caption).lineLimit(1).truncationMode(.middle)
                        if let last = Format.relative(project.lastActivity) {
                            Text("· modifié \(last)").textStyle(.caption)
                        }
                    }
                }
                Spacer(minLength: 12)
                HoverReveal {
                    IconButton(icon: .arrowUpRight, help: "Afficher dans le Finder") { SystemTrash.reveal(project.path) }
                }
                VStack(alignment: .trailing, spacing: 2) {
                    Text(Format.bytes(project.bytes)).textStyle(.bodyStrong).monospacedDigit()
                    if project.regenerableBytes > 0 {
                        Text("dont \(Format.bytes(project.regenerableBytes)) recréables").textStyle(.caption).monospacedDigit()
                    }
                }
            }
        }
        .shimmering(activity == .working)
        .opacity(activity == .queued ? 0.55 : 1)
        .animation(Motion.snappy, value: activity)
        .accessibilityElement(children: .contain)
    }
}
