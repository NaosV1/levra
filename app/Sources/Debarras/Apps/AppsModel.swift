import AppKit
import DebarrasBindings
import Observation

@MainActor
@Observable
final class AppsModel {
    enum Sort: String, CaseIterable, Identifiable {
        case size = "Taille"
        case name = "Nom"
        case lastUsed = "Dernière ouverture"
        var id: Self { self }
    }

    /// An uninstall being prepared or shown.
    struct Plan {
        let session: UninstallSession
        let app: AppRecord
        let appPresent: Bool
        let items: [LeftoverInfo]
        let brewCask: String?
        var selection: Set<String>
        var useBrew: Bool
        var running: Bool
        var outcome: CleanupOutcome?
        var working = false

        var selectedBytes: UInt64 {
            items.filter { selection.contains($0.id) }.reduce(0) { $0 + $1.bytes }
        }
    }

    private(set) var apps: [AppRecord] = []
    /// Whether `apps` holds a real listing yet (it is empty until then).
    private(set) var hasLoaded = false
    private(set) var loading = false
    private(set) var error: String?
    var search = ""
    var sort: Sort = .size
    var plan: Plan?
    /// Bundle id of a removed app waiting for its plan (from a notification).
    private var removedApps: [String: AppRecord] = [:]

    var visibleApps: [AppRecord] {
        let filtered = search.isEmpty
            ? apps
            : apps.filter { $0.name.localizedCaseInsensitiveContains(search) || $0.bundleId.localizedCaseInsensitiveContains(search) }
        switch sort {
        case .size: return filtered.sorted { $0.bytes > $1.bytes }
        case .name: return filtered.sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
        case .lastUsed: return filtered.sorted { ($0.lastUsed ?? 0) < ($1.lastUsed ?? 0) }
        }
    }

    func load() async {
        loading = true
        error = nil
        do {
            apps = try await background { try listApps() }
            hasLoaded = true
        } catch {
            self.error = error.message
        }
        loading = false
    }

    func replaceList(_ apps: [AppRecord]) {
        self.apps = apps
        hasLoaded = true
    }

    func dismissError() {
        error = nil
    }

    /// Prepares the uninstall of an app from the list, a dropped bundle,
    /// or one that was already removed.
    func prepare(_ app: AppRecord) async {
        let installed = apps
        do {
            let session = try await background { try planUninstall(app: app, installed: installed) }
            let items = session.items()
            plan = Plan(
                session: session,
                app: session.app(),
                appPresent: session.appPresent(),
                items: items,
                brewCask: session.brewCask(),
                selection: Set(items.filter(\.selected).map(\.id)),
                useBrew: session.brewCask() != nil,
                running: session.isAppRunning()
            )
        } catch {
            self.error = error.message
        }
    }

    func prepareDropped(_ url: URL) async {
        do {
            let app = try await background { try inspectApp(path: url.path) }
            await prepare(app)
        } catch {
            self.error = error.message
        }
    }

    /// Keeps the identity of an app removed outside Débarras.
    func noteRemoved(_ app: AppRecord) {
        removedApps[app.bundleId] = app
    }

    func prepareRemoved(bundleId: String) async {
        guard let app = removedApps[bundleId] else { return }
        await prepare(app)
    }

    func quitApp() async {
        guard let bundleId = plan?.app.bundleId else { return }
        NSRunningApplication.runningApplications(withBundleIdentifier: bundleId).forEach { $0.terminate() }
        try? await Task.sleep(for: .seconds(2))
        if let session = plan?.session {
            plan?.running = await background { session.isAppRunning() }
        }
    }

    func uninstall() async {
        guard let plan, !plan.selection.isEmpty else { return }
        self.plan?.working = true
        let ids = Array(plan.selection)
        let useBrew = plan.useBrew
        let session = plan.session
        let result = await background { session.execute(ids: ids, useBrew: useBrew) }
        self.plan?.outcome = await CleanupOutcome.finishing(result)
        self.plan?.working = false
        removedApps[plan.app.bundleId] = nil
        await load()
    }
}
