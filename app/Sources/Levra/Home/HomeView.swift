import LevraBindings
import SwiftUI

/// The first page: how full the disk is, and one card per thing worth
/// doing, each leading to its page.
struct HomeView: View {
    @Environment(AppModel.self) private var app
    @State private var confirmingEmpty = false
    @State private var emptyRefusal: String?

    var body: some View {
        PageScroll {
            PageHeader(title: "Accueil", subtitle: "L'essentiel de ton Mac, et ce que tu peux faire.") {}
        } content: {
            VStack(alignment: .leading, spacing: 20) {
                if let disk = app.disk {
                    DiskHero(disk: disk)
                }
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 300), spacing: 14)], spacing: 14) {
                    cleanupCard
                    appsCard
                    trashCard
                    memoryCard
                    backgroundCard
                }
            }
        }
        // Runs again once the apps are listed, which the background card needs.
        .task(id: app.apps.hasLoaded) {
            app.refreshTrash()
            if app.apps.hasLoaded, app.debloat.session == nil, app.debloat.phase == .idle {
                await app.debloat.load(installed: app.apps.apps)
            }
        }
        .confirmationDialog("Vider la corbeille ?", isPresented: $confirmingEmpty, titleVisibility: .visible) {
            Button("Vider la corbeille", role: .destructive) { emptyTrash() }
        } message: {
            Text("Tout ce qu'elle contient est supprimé définitivement. Le Finder demandera ton mot de passe si des éléments sont protégés.")
        }
        .alert("La corbeille n'a pas été vidée", isPresented: Binding(get: { emptyRefusal != nil }, set: { if !$0 { emptyRefusal = nil } })) {
            Button("OK") { emptyRefusal = nil }
        } message: {
            Text(emptyRefusal ?? "")
        }
    }

    private func open(_ section: AppModel.Section) {
        withAnimation(Motion.glide) { app.section = section }
    }

    @ViewBuilder
    private var cleanupCard: some View {
        let cleanup = app.cleanup
        switch cleanup.phase {
        case .idle:
            HomeCard(icon: .clean, title: "Nettoyage", value: "À analyser", caption: "Trouve ce qui peut partir sans risque.") {
                Button("Analyser mon Mac") { Task { await cleanup.scan() } }
                    .buttonStyle(.app(.primary))
            }
        case let .scanning(_, _, done):
            let finished = done.nonzeroBitCount
            HomeCard(
                icon: .clean,
                title: "Nettoyage",
                value: "Analyse…",
                caption: "\(finished) \(Format.agree(finished, "vérification terminée", "vérifications terminées")) sur \(CleanupModel.steps.count)."
            ) {
                Spinner(size: 16)
            }
        case .ready, .cleaning:
            HomeCard(
                icon: .clean,
                title: "Nettoyage",
                value: Format.bytes(cleanup.recommendedBytes),
                caption: "à libérer sans risque.",
                tone: cleanup.recommendedBytes > 0 ? .success : nil
            ) {
                Button("Voir") { open(.cleanup) }.buttonStyle(.app(.secondary))
            }
        }
    }

    private var appsCard: some View {
        let unused = app.apps.unusedApps
        return HomeCard(
            icon: .grid,
            title: "Apps inutilisées",
            value: app.apps.hasLoaded ? Format.counted(unused.count, "app") : "…",
            caption: "pas ouvertes depuis 6 mois · \(Format.bytes(unused.reduce(0) { $0 + $1.bytes }))."
        ) {
            Button("Voir") {
                app.apps.filter = .unused
                open(.apps)
            }
            .buttonStyle(.app(.secondary))
            .disabled(unused.isEmpty)
        }
    }

    private var trashCard: some View {
        HomeCard(
            icon: .delete,
            title: "Corbeille",
            value: app.trashBytes.map(Format.bytes) ?? "Taille inconnue",
            caption: app.trashBytes == nil
                ? "Visible avec l'accès complet au disque ; le Finder peut la vider quand même."
                : "supprimés définitivement en la vidant."
        ) {
            if app.emptyingTrash {
                Spinner(size: 16)
            } else {
                Button("Vider…") { confirmingEmpty = true }
                    .buttonStyle(.app(.secondary))
                    .disabled(app.trashBytes == 0)
            }
        }
    }

    private var memoryCard: some View {
        let stats = app.memoryStats
        return HomeCard(
            icon: .chip,
            title: "Mémoire",
            value: stats.map { Format.bytes($0.used) } ?? "…",
            caption: stats.map { "utilisés sur \(Format.bytes($0.total)) · pression \($0.pressure.label.lowercased())." } ?? "",
            tone: stats?.pressure.tone
        ) {
            Button("Voir") { open(.memory) }.buttonStyle(.app(.secondary))
        }
    }

    private var backgroundCard: some View {
        let debloat = app.debloat
        let stray = debloat.strayItems.count
        return HomeCard(
            icon: .activity,
            title: "En arrière-plan",
            value: debloat.session == nil ? "…" : Format.counted(debloat.items.count, "élément"),
            caption: debloat.session == nil
                ? "Ce que les apps lancent sans te demander."
                : "dont \(stray) sans app installée.",
            tone: stray > 0 ? .warning : nil
        ) {
            Button("Voir") { open(.debloat) }.buttonStyle(.app(.secondary))
        }
    }

    private func emptyTrash() {
        Task { emptyRefusal = await app.emptyTrash() }
    }
}

/// How full the disk is, as a ring and in words.
private struct DiskHero: View {
    let disk: DiskSpace

    var body: some View {
        HStack(spacing: 24) {
            DiskRing(disk: disk)
            VStack(alignment: .leading, spacing: 6) {
                Text("\(Format.bytes(UInt64(max(disk.available, 0)))) libres")
                    .font(.system(size: 30, weight: .semibold))
                    .tracking(-0.8)
                    .foregroundStyle(Theme.textPrimary)
                    .contentTransition(.numericText())
                Text("\(Format.bytes(UInt64(max(disk.total - disk.available, 0)))) utilisés sur \(Format.bytes(UInt64(max(disk.total, 0)))).")
                    .textStyle(.secondary)
            }
            Spacer(minLength: 0)
        }
        .padding(24)
        .card()
        .accessibilityElement(children: .combine)
    }
}

/// One thing worth doing: what it is, the figure that says if it matters,
/// and the way there.
private struct HomeCard<Actions: View>: View {
    let icon: Papicon
    let title: String
    let value: String
    let caption: String
    var tone: Badge.Tone?
    @ViewBuilder var actions: Actions

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 8) {
                Icon(icon, size: 16).foregroundStyle(Theme.textSecondary)
                Text(title).textStyle(.label)
                Spacer()
                if let tone {
                    Circle().fill(tone.color).frame(width: 8, height: 8)
                }
            }
            VStack(alignment: .leading, spacing: 4) {
                Text(value)
                    .font(.system(size: 24, weight: .semibold).monospacedDigit())
                    .tracking(-0.5)
                    .foregroundStyle(Theme.textPrimary)
                    .contentTransition(.numericText())
                Text(caption)
                    .textStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 0)
            HStack { actions }
        }
        .padding(18)
        .frame(maxWidth: .infinity, minHeight: 190, alignment: .topLeading)
        .card()
    }
}
