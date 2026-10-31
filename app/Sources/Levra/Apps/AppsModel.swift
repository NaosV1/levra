import AppKit
import LevraBindings
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

    enum Filter: String, CaseIterable, Identifiable {
        case all = "Toutes"
        case unused = "Inutilisées"
        var id: Self { self }
    }

    /// Not opened for this long, or never according to Spotlight.
    static let unusedAfter: TimeInterval = 180 * 86_400

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
    var filter: Filter = .all
    var plan: Plan?
    /// Paths of the apps picked for uninstalling together.
    var bulkSelection: Set<String> = []
    private(set) var bulkWorking = false
    private(set) var bulkOutcome: CleanupOutcome?
    /// Bundle id of a removed app waiting for its plan (from a notification).
    private var removedApps: [String: AppRecord] = [:]

    var unusedApps: [AppRecord] {
        let limit = UInt64(max(0, Date().timeIntervalSince1970 - Self.unusedAfter))
        return apps.filter { ($0.lastUsed ?? 0) < limit }
    }

    var visibleApps: [AppRecord] {
        let scope = filter == .unused ? unusedApps : apps
        let filtered = search.isEmpty
            ? scope
            : scope.filter { $0.name.localizedCaseInsensitiveContains(search) || $0.bundleId.localizedCaseInsensitiveContains(search) }
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

    /// Keeps the identity of an app removed outside Levra.
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

    /// Uninstalls every picked app with its default selection of leftovers,
    /// then hands what was locked to the Finder in one go: one password for
    /// the whole batch.
    func uninstallSelected() async {
        let targets = apps.filter { bulkSelection.contains($0.path) }
        guard !targets.isEmpty else { return }
        let installed = apps
        bulkWorking = true
        let merged = await background { () -> ExecutionResult in
            var merged = ExecutionResult(completed: [], freedBytes: 0, needsAuthorization: [], failures: [])
            for app in targets {
                do {
                    let session = try planUninstall(app: app, installed: installed)
                    let ids = session.items().filter(\.selected).map(\.id)
                    let result = session.execute(ids: ids, useBrew: session.brewCask() != nil)
                    merged.completed += result.completed
                    merged.freedBytes += result.freedBytes
                    merged.needsAuthorization += result.needsAuthorization
                    merged.failures += result.failures
                } catch {
                    merged.failures.append(FailureInfo(target: app.name, reason: error.message))
                }
            }
            return merged
        }
        bulkOutcome = await CleanupOutcome.finishing(merged)
        bulkSelection = []
        bulkWorking = false
        await load()
    }

    func dismissBulkOutcome() {
        bulkOutcome = nil
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
