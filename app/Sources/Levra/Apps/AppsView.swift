import LevraBindings
import SwiftUI

struct AppsView: View {
    @Bindable var model: AppsModel
    @Environment(AppModel.self) private var app
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var dropTargeted = false
    @State private var confirmingBulk = false

    var body: some View {
        PageScroll {
            PageHeader(title: "Applications", subtitle: subtitle) {
                SearchField(prompt: "Rechercher une app", text: $model.search)
                SegmentedControl(
                    options: AppsModel.Sort.allCases.map { ($0, $0.rawValue) },
                    selection: $model.sort
                )
                IconButton(icon: .refresh, help: "Actualiser") { Task { await model.load() } }
                    .keyboardShortcut("r", modifiers: .command)
                    .disabled(model.loading)
            }
        } content: {
            VStack(alignment: .leading, spacing: 16) {
                if let outcome = model.bulkOutcome {
                    OutcomeBanner(outcome: outcome) { withAnimation(Motion.snappy) { model.dismissBulkOutcome() } }
                        .transition(.lift(reduceMotion: reduceMotion))
                }
                dropZone
                if model.loading && model.apps.isEmpty {
                    PageLoading(icon: .grid, label: "Liste des applications…")
                } else {
                    HStack(spacing: 12) {
                        SegmentedControl(
                            options: [
                                (AppsModel.Filter.all, "Toutes · \(model.apps.count)"),
                                (.unused, "Inutilisées · \(model.unusedApps.count)"),
                            ],
                            selection: $model.filter
                        )
                        if model.filter == .unused {
                            Text("Pas ouvertes depuis 6 mois, ou jamais selon Spotlight, qui ne voit pas tout : vérifie avant de cocher.")
                                .textStyle(.caption)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    }
                    let apps = model.visibleApps
                    VStack(spacing: 0) {
                        ForEach(Array(apps.enumerated()), id: \.element.path) { index, app in
                            AppRow(
                                app: app,
                                isSelected: model.filter == .unused ? selection(of: app) : nil,
                                isLast: index == apps.count - 1
                            ) { Task { await model.prepare(app) } }
                            .staggeredAppear(index: index)
                        }
                    }
                    .card()
                    .animation(Motion.snappy, value: apps.map(\.path))
                }
            }
            .animation(Motion.snappy, value: model.bulkOutcome)
        }
        .safeAreaInset(edge: .bottom) {
            if model.filter == .unused, !model.bulkSelection.isEmpty {
                ActionBar {
                    Text(Format.counted(model.bulkSelection.count, "app")).textStyle(.secondary)
                    Text(Format.bytes(selectedBytes)).textStyle(.bodyStrong).monospacedDigit().contentTransition(.numericText())
                    Spacer()
                    if model.bulkWorking {
                        Spinner(size: 14)
                    }
                    Button("Tout désélectionner") { withAnimation(Motion.quick) { model.bulkSelection = [] } }
                        .buttonStyle(.app(.ghost))
                    Button { confirmingBulk = true } label: {
                        ButtonLabel("Désinstaller \(Format.counted(model.bulkSelection.count, "app"))", icon: .delete)
                    }
                    .buttonStyle(.app(.danger))
                    .disabled(model.bulkWorking)
                }
                .animation(Motion.snappy, value: selectedBytes)
                .transition(.move(edge: .bottom).combined(with: .opacity))
            }
        }
        .animation(Motion.snappy, value: model.bulkSelection.isEmpty)
        .confirmationDialog(
            "Désinstaller \(Format.counted(model.bulkSelection.count, "app")) ?",
            isPresented: $confirmingBulk,
            titleVisibility: .visible
        ) {
            Button("Désinstaller", role: .destructive) {
                Task {
                    await model.uninstallSelected()
                    app.refreshDisk()
                }
            }
        } message: {
            Text("Chaque app part à la corbeille avec ses restes sûrs, comme dans la revue de désinstallation. Le Finder demandera ton mot de passe une seule fois si besoin.")
        }
        .dropDestination(for: URL.self) { urls, _ in
            guard let url = urls.first(where: { $0.pathExtension == "app" }) else { return false }
            Task { await model.prepareDropped(url) }
            return true
        } isTargeted: { targeted in
            withAnimation(Motion.snappy) { dropTargeted = targeted }
        }
        .uninstallReview(model)
    }

    private var subtitle: String {
        let total = model.apps.reduce(0) { $0 + $1.bytes }
        return "\(Format.counted(model.apps.count, "app")) · \(Format.bytes(total))"
    }

    private var selectedBytes: UInt64 {
        model.apps.filter { model.bulkSelection.contains($0.path) }.reduce(0) { $0 + $1.bytes }
    }

    private func selection(of app: AppRecord) -> Binding<Bool> {
        Binding(
            get: { model.bulkSelection.contains(app.path) },
            set: { if $0 { model.bulkSelection.insert(app.path) } else { model.bulkSelection.remove(app.path) } }
        )
    }

    private var dropZone: some View {
        HStack(spacing: 12) {
            Icon(.download, size: 18)
                .foregroundStyle(dropTargeted ? Theme.accent : Theme.textTertiary)
            VStack(alignment: .leading, spacing: 2) {
                Text("Glisse une app ici pour la désinstaller complètement").textStyle(.bodyStrong)
                Text("L'app et tout ce qu'elle a laissé : préférences, caches, conteneurs, services.").textStyle(.secondary)
            }
            Spacer()
        }
        .padding(16)
        .background(dropTargeted ? Theme.accent.opacity(0.06) : .clear, in: RoundedRectangle(cornerRadius: Theme.radius))
        .overlay {
            RoundedRectangle(cornerRadius: Theme.radius)
                .strokeBorder(dropTargeted ? Theme.accent : Theme.borderStrong, style: StrokeStyle(lineWidth: 1, dash: [4, 4]))
        }
        .scaleEffect(dropTargeted ? 1.01 : 1)
    }
}

private struct AppRow: View {
    let app: AppRecord
    /// Set when apps can be picked for uninstalling together.
    var isSelected: Binding<Bool>?
    let isLast: Bool
    let onUninstall: () -> Void

    var body: some View {
        CardRow(isLast: isLast) {
            HStack(spacing: 12) {
                if let isSelected {
                    Checkbox(isOn: isSelected)
                }
                AppIcon(path: app.path, size: 28)
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 6) {
                        Text(app.name).textStyle(.bodyStrong).lineLimit(1)
                        if let version = app.version {
                            Text(version).textStyle(.caption)
                        }
                    }
                    Text(app.lastUsedLabel).textStyle(.secondary)
                }
                Spacer(minLength: 12)
                HoverReveal {
                    HStack(spacing: 4) {
                        IconButton(icon: .arrowUpRight, help: "Afficher dans le Finder") { SystemTrash.reveal(app.path) }
                        Button("Désinstaller…", action: onUninstall).buttonStyle(.app(.secondary, size: .small))
                    }
                }
                Text(Format.bytes(app.bytes))
                    .textStyle(.bodyStrong)
                    .monospacedDigit()
                    .frame(width: 76, alignment: .trailing)
            }
        }
        .onTapGesture(perform: onUninstall)
        .contextMenu {
            Button("Désinstaller…", action: onUninstall)
            Button("Afficher dans le Finder") { SystemTrash.reveal(app.path) }
        }
        .accessibilityElement(children: .contain)
        .accessibilityAction(named: "Désinstaller…", onUninstall)
        .accessibilityAction(named: "Afficher dans le Finder") { SystemTrash.reveal(app.path) }
    }
}
