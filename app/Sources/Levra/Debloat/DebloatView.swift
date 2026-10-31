import LevraBindings
import SwiftUI

struct DebloatView: View {
    let model: DebloatModel
    let apps: AppsModel
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var confirming = false

    var body: some View {
        PageScroll {
            PageHeader(title: "Allègement", subtitle: subtitle) {
                IconButton(icon: .refresh, help: "Actualiser") { reload() }
                    .keyboardShortcut("r", modifiers: .command)
                    .disabled(busy || !apps.hasLoaded)
            }
        } content: {
            if model.session != nil {
                results
            } else if let error = model.error ?? (apps.hasLoaded ? nil : apps.error) {
                EmptyState(icon: .alert, title: "Analyse impossible", message: error) {
                    Button("Réessayer") {
                        if apps.hasLoaded { reload() } else { Task { await apps.load() } }
                    }
                    .buttonStyle(.app(.secondary))
                }
            } else {
                PageLoading(icon: .feather, label: "Lecture des agents et services…")
            }
        }
        .safeAreaInset(edge: .bottom) { actionBar }
        .animation(Motion.snappy, value: model.phase)
        .confirmationDialog(
            "Mettre \(Format.counted(model.selection.count, "élément")) à la corbeille ?",
            isPresented: $confirming,
            titleVisibility: .visible
        ) {
            Button("Mettre à la corbeille", role: .destructive) {
                Task { await model.remove(installed: apps.apps) }
            }
        } message: {
            Text("Ils ne seront plus lancés. Un service système déjà chargé s'arrête au prochain redémarrage.")
        }
        .uninstallReview(apps) { reload() }
        // Jobs are attributed to the installed apps: wait for their list.
        .task(id: apps.hasLoaded) {
            guard apps.hasLoaded, model.phase == .idle, model.session == nil else { return }
            await model.load(installed: apps.apps)
        }
    }

    private var busy: Bool {
        model.phase == .loading || model.phase == .removing
    }

    private var proposed: String {
        let count = model.items.filter(\.selected).count
        return "\(count) \(Format.agree(count, "proposé", "proposés")) à la suppression"
    }

    private var subtitle: String {
        guard model.session != nil else { return "Ce que les apps lancent en arrière-plan, et les apps d'Apple." }
        return "\(model.items.count) agents et services · \(model.strayItems.count) sans app installée"
    }

    private func reload() {
        Task { await model.load(installed: apps.apps) }
    }

    private func uninstall(_ app: AppRecord) {
        Task { await apps.prepare(app) }
    }

    private var results: some View {
        VStack(alignment: .leading, spacing: 20) {
            if let outcome = model.outcome {
                OutcomeBanner(outcome: outcome) { withAnimation(Motion.snappy) { model.dismissOutcome() } }
                    .transition(.lift(reduceMotion: reduceMotion))
            }
            HStack(spacing: 12) {
                StatTile(
                    label: "En arrière-plan",
                    value: "\(model.items.count)",
                    detail: "\(model.loadedCount) \(Format.agree(model.loadedCount, "chargé", "chargés")) en ce moment"
                )
                StatTile(
                    label: "Sans app installée",
                    value: "\(model.strayItems.count)",
                    detail: model.strayItems.isEmpty ? "Rien à retirer" : proposed,
                    tone: model.strayItems.isEmpty ? .success : .warning
                )
                StatTile(
                    label: "Apps intégrées à macOS",
                    value: "\(model.systemAppCount)",
                    detail: "\(Format.bytes(model.systemAppBytes)), protégées par macOS"
                )
            }
            straySection
            ownedSection
            appleSection
        }
        .animation(Motion.snappy, value: model.items.map(\.id))
    }

    private var straySection: some View {
        VStack(alignment: .leading, spacing: 8) {
            SectionHeader(title: "Sans app installée", icon: .delete, trailing: "\(model.strayItems.count)")
            if model.strayItems.isEmpty {
                Text("Chaque agent et service appartient à une app installée.")
                    .textStyle(.secondary)
                    .padding(14)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .card()
            } else {
                rows(model.strayItems)
            }
        }
    }

    private var ownedSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            SectionHeader(title: "Installés par tes apps", icon: .activity, trailing: "\(model.ownedItems.count)")
            if !model.ownedItems.isEmpty {
                rows(model.ownedItems)
            }
            HStack(spacing: 8) {
                Text("Ils partent avec leur app. Pour qu'une app ne tourne plus en arrière-plan sans la désinstaller, désactive-la dans les réglages d'ouverture de macOS.")
                    .textStyle(.caption)
                    .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 12)
                Button("Ouvrir les réglages") { LoginItemsPane.open() }
                    .buttonStyle(.app(.ghost, size: .small))
            }
            .padding(.horizontal, 4)
        }
    }

    private var appleSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            SectionHeader(title: "Apps d'Apple", icon: .grid)
            Banner(
                tone: .neutral,
                icon: .lock,
                title: "Bourse, News, Astuces… ne peuvent pas être supprimées",
                message: "Les \(model.systemAppCount) apps intégrées à macOS (\(Format.bytes(model.systemAppBytes)) en tout) sont sur le volume système scellé : en lecture seule, et réinstallées à chaque mise à jour. Les supprimer obligerait à désactiver la protection du système (SIP), ce que Levra ne fait pas. Pour ne plus les voir, retire-les du Dock."
            )
            if model.appleApps.isEmpty {
                Text("GarageBand, iMovie, Pages, Numbers et Keynote, les apps d'Apple que l'on peut supprimer, ne sont pas installées.")
                    .textStyle(.caption)
                    .padding(.horizontal, 4)
            } else {
                VStack(spacing: 0) {
                    ForEach(Array(model.appleApps.enumerated()), id: \.element.path) { index, app in
                        AppleAppRow(app: app, isLast: index == model.appleApps.count - 1) { uninstall(app) }
                    }
                }
                .card()
            }
        }
    }

    private func rows(_ items: [BackgroundItemInfo]) -> some View {
        VStack(spacing: 0) {
            ForEach(Array(items.enumerated()), id: \.element.id) { index, item in
                BackgroundRow(
                    item: item,
                    isSelected: binding(for: item),
                    isLast: index == items.count - 1,
                    onUninstall: uninstall
                )
                .staggeredAppear(index: index)
            }
        }
        .card()
    }

    @ViewBuilder
    private var actionBar: some View {
        if model.session != nil, !model.strayItems.isEmpty {
            ActionBar {
                Text("\(model.selection.count) \(Format.agree(model.selection.count, "sélectionné", "sélectionnés"))")
                    .textStyle(.secondary)
                    .monospacedDigit()
                    .contentTransition(.numericText())
                Spacer()
                Button("Sélection proposée") { withAnimation(Motion.quick) { model.selectProposed() } }
                    .buttonStyle(.app(.ghost))
                Button { confirming = true } label: { ButtonLabel("Mettre à la corbeille", icon: .delete) }
                    .buttonStyle(.app(.danger))
                    .disabled(model.selection.isEmpty || busy)
            }
            .animation(Motion.snappy, value: model.selection.count)
            .transition(.move(edge: .bottom).combined(with: .opacity))
        }
    }

    private func binding(for item: BackgroundItemInfo) -> Binding<Bool> {
        Binding(
            get: { model.selection.contains(item.id) },
            set: { model.setSelected($0, item) }
        )
    }
}

private struct BackgroundRow: View {
    let item: BackgroundItemInfo
    @Binding var isSelected: Bool
    let isLast: Bool
    let onUninstall: (AppRecord) -> Void

    var body: some View {
        CardRow(isLast: isLast) {
            HStack(spacing: 12) {
                if item.removable {
                    Checkbox(isOn: $isSelected)
                }
                icon.frame(width: 24, height: 24)
                VStack(alignment: .leading, spacing: 3) {
                    HStack(spacing: 6) {
                        if item.needsAuthorization {
                            Icon(.lock, size: 12)
                                .foregroundStyle(Theme.textTertiary)
                                .help("macOS demandera ton autorisation pour le retirer.")
                        }
                        Text(item.label).textStyle(.bodyStrong).lineLimit(1).truncationMode(.middle)
                    }
                    Text(details)
                        .textStyle(.secondary)
                        .lineLimit(1)
                        .help(item.apps.map(\.name).joined(separator: ", "))
                }
                Spacer(minLength: 12)
                HoverReveal {
                    HStack(spacing: 4) {
                        IconButton(icon: .arrowUpRight, help: "Afficher dans le Finder") { SystemTrash.reveal(item.path) }
                        if let owner = soleOwner {
                            Button("Désinstaller \(owner.name)…") { onUninstall(owner) }
                                .buttonStyle(.app(.secondary, size: .small))
                        }
                    }
                }
                status
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityActions {
            Button("Afficher dans le Finder") { SystemTrash.reveal(item.path) }
            if let owner = soleOwner {
                Button("Désinstaller \(owner.name)…") { onUninstall(owner) }
            }
        }
    }

    /// Uninstalling is offered only when one app owns the job.
    private var soleOwner: AppRecord? {
        item.apps.count == 1 ? item.apps.first : nil
    }

    @ViewBuilder
    private var icon: some View {
        if let app = soleOwner {
            AppIcon(path: app.path, size: 24)
        } else {
            Icon(iconName, size: 16).foregroundStyle(Theme.textSecondary)
        }
    }

    /// Several apps of one publisher share the job: none of their icons fits.
    private var iconName: Papicon {
        switch item.owner {
        case .apps: .grid
        case .homebrew: .package
        case .noApp, .broken: .activity
        }
    }

    /// Who it belongs to, or why it can go, then when it runs.
    private var details: String {
        [reason, item.schedule.label(scope: item.scope), item.scope.label].joined(separator: " · ")
    }

    private var reason: String {
        switch item.owner {
        // Lié, not used by: the link may be only a shared publisher.
        case .apps: "Lié à \(ownerNames)"
        case .homebrew: "Service Homebrew (brew services)"
        case .noApp: "Aucune app de cet éditeur installée"
        case .broken: item.program == nil ? "Fichier vide, ne lance rien" : "Le programme qu'il lance n'existe plus"
        }
    }

    private var ownerNames: String {
        let names = item.apps.map(\.name)
        guard names.count > 2 else { return names.joined(separator: " et ") }
        return "\(names[0]), \(names[1]) et \(Format.counted(names.count - 2, "autre"))"
    }

    @ViewBuilder
    private var status: some View {
        switch item.loaded {
        case true?:
            Badge(text: "Actif", tone: .accent)
                .help("macOS l'a chargé : il tourne, ou attend son heure.")
        case false?:
            Badge(text: "Inactif", tone: .neutral)
                .help("macOS ne l'a pas chargé : il ne tourne pas en ce moment.")
        case nil:
            EmptyView()
        }
    }
}

private struct AppleAppRow: View {
    let app: AppRecord
    let isLast: Bool
    let onUninstall: () -> Void

    var body: some View {
        CardRow(isLast: isLast) {
            HStack(spacing: 12) {
                AppIcon(path: app.path, size: 28)
                VStack(alignment: .leading, spacing: 2) {
                    Text(app.name).textStyle(.bodyStrong).lineLimit(1)
                    Text(app.lastUsedLabel).textStyle(.secondary)
                }
                Spacer(minLength: 12)
                Button("Désinstaller…", action: onUninstall)
                    .buttonStyle(.app(.secondary, size: .small))
                Text(Format.bytes(app.bytes))
                    .textStyle(.bodyStrong)
                    .monospacedDigit()
                    .frame(width: 76, alignment: .trailing)
            }
        }
    }
}
