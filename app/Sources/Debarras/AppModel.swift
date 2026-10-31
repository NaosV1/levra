import DebarrasBindings
import Foundation
import Observation

/// Root state: navigation, disk space, settings, and the background
/// services (uninstall detection, low-space alerts).
@MainActor
@Observable
final class AppModel {
    enum Section: String, CaseIterable, Identifiable {
        case cleanup, explorer, largeFiles, duplicates, memory, apps, debloat

        var id: Self { self }

        var title: String {
            switch self {
            case .cleanup: "Nettoyage"
            case .explorer: "Explorateur"
            case .largeFiles: "Gros fichiers"
            case .duplicates: "Doublons"
            case .memory: "Mémoire"
            case .apps: "Applications"
            case .debloat: "Allègement"
            }
        }

        var icon: Papicon {
            switch self {
            case .cleanup: .clean
            case .explorer: .folder
            case .largeFiles: .file
            case .duplicates: .copy
            case .memory: .chip
            case .apps: .grid
            case .debloat: .feather
            }
        }
    }

    private enum Keys {
        static let watchUninstalls = "watchUninstalls"
        static let lowSpaceGB = "lowSpaceGB"
    }

    var section: Section = .cleanup
    private(set) var disk: DiskSpace?
    /// System memory figures, refreshed every minute (and by the memory view).
    private(set) var memoryStats: MemoryStatsInfo?

    let cleanup = CleanupModel()
    let explorer = ExplorerModel()
    let duplicates = DuplicatesModel()
    let memory = MemoryModel()
    let apps = AppsModel()
    let debloat = DebloatModel()

    var watchUninstalls: Bool {
        didSet {
            UserDefaults.standard.set(watchUninstalls, forKey: Keys.watchUninstalls)
            watchUninstalls ? startWatching() : watcher.stop()
        }
    }

    /// Below this many gigabytes free, a notification is posted once.
    var lowSpaceGB: Int {
        didSet { UserDefaults.standard.set(lowSpaceGB, forKey: Keys.lowSpaceGB) }
    }

    private let notifier = Notifier()
    private let watcher = AppRemovalWatcher()
    private var started = false
    private var warnedLowSpace = false
    private var warnedMemory = false

    init() {
        let defaults = UserDefaults.standard
        watchUninstalls = defaults.object(forKey: Keys.watchUninstalls) as? Bool ?? true
        lowSpaceGB = defaults.object(forKey: Keys.lowSpaceGB) as? Int ?? 20
    }

    func start() async {
        guard !started else { return }
        started = true
        notifier.activate()
        notifier.onOpen = { [weak self] target in
            guard let self else { return }
            switch target {
            case .cleanup:
                section = .cleanup
            case .memory:
                section = .memory
            case let .uninstall(bundleId):
                section = .apps
                Task { await self.apps.prepareRemoved(bundleId: bundleId) }
            }
        }
        memory.onRefresh = { [weak self] stats in self?.memoryStats = stats }
        watcher.onRemoval = { [weak self] app in self?.appRemoved(app) }
        watcher.onListChanged = { [weak self] list in self?.apps.replaceList(list) }

        refreshDisk()
        refreshMemory()
        Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(60))
                self?.refreshDisk()
                self?.refreshMemory()
            }
        }
        await apps.load()
        if watchUninstalls {
            startWatching()
        }
    }

    func refreshDisk() {
        disk = DiskSpace.current()
        guard let disk else { return }
        let threshold = Int64(lowSpaceGB) * 1_000_000_000
        if disk.available < threshold, !warnedLowSpace {
            warnedLowSpace = true
            notifier.lowSpace(available: disk.available)
        } else if disk.available >= threshold {
            warnedLowSpace = false
        }
    }

    /// Also warns once per episode when memory pressure turns critical,
    /// naming what uses the most.
    func refreshMemory() {
        Task {
            guard let stats = try? await background({ try DebarrasBindings.memoryStats() }) else { return }
            memoryStats = stats
            guard stats.pressure == .critical else {
                warnedMemory = false
                return
            }
            guard !warnedMemory else { return }
            warnedMemory = true
            let top = try? await background({ try memorySnapshot() }).groups.first { $0.quittable }
            notifier.memoryCritical(topName: top?.name, topBytes: top?.footprint)
        }
    }

    private func startWatching() {
        let folders = [URL(fileURLWithPath: "/Applications"), URL(fileURLWithPath: NSHomeDirectory()).appending(path: "Applications")]
        watcher.start(folders: folders, apps: apps.apps)
    }

    /// An app disappeared without Débarras: tell the user what it left.
    private func appRemoved(_ app: AppRecord) {
        apps.noteRemoved(app)
        let installed = apps.apps
        Task {
            guard let session = try? await background({ try planUninstall(app: app, installed: installed) }) else { return }
            let items = session.items()
            guard !items.isEmpty else { return }
            notifier.appRemoved(
                name: app.name,
                bundleId: app.bundleId,
                leftovers: items.count,
                bytes: items.reduce(0) { $0 + $1.bytes }
            )
        }
        refreshDisk()
    }
}
