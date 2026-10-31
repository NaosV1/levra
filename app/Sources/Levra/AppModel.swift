import LevraBindings
import Foundation
import Observation

/// Root state: navigation, disk space, settings, and the background
/// services (uninstall detection, low-space alerts).
@MainActor
@Observable
final class AppModel {
    enum Section: String, CaseIterable, Identifiable {
        case home, cleanup, explorer, largeFiles, duplicates, photos, archive, memory, apps, debloat

        var id: Self { self }

        var title: String {
            switch self {
            case .home: "Accueil"
            case .cleanup: "Nettoyage"
            case .explorer: "Explorateur"
            case .largeFiles: "Gros fichiers"
            case .duplicates: "Doublons"
            case .photos: "Photos similaires"
            case .archive: "Vieux projets"
            case .memory: "Mémoire"
            case .apps: "Applications"
            case .debloat: "Allègement"
            }
        }

        /// What the page is for, in a sentence: shown on hover in the sidebar.
        var summary: String {
            switch self {
            case .home: "L'essentiel de ton Mac en un coup d'œil."
            case .cleanup: "Ce qui peut partir sans risque : caches, fichiers qui se recréent tout seuls."
            case .explorer: "Où va l'espace, dossier par dossier."
            case .largeFiles: "Les fichiers les plus lourds, pour repérer les oubliés."
            case .duplicates: "Les fichiers présents en plusieurs exemplaires."
            case .photos: "Les photos qui se ressemblent : rafales, reprises, copies."
            case .archive: "Les projets oubliés, compressés ou rangés sur un autre disque."
            case .memory: "Ce qui occupe la mémoire vive, en direct."
            case .apps: "Tes apps, et leur désinstallation complète."
            case .debloat: "Ce que les apps lancent en arrière-plan, et les apps d'Apple."
            }
        }

        var icon: Papicon {
            switch self {
            case .home: .home
            case .cleanup: .clean
            case .explorer: .folder
            case .largeFiles: .file
            case .duplicates: .copy
            case .photos: .layers
            case .archive: .package
            case .memory: .chip
            case .apps: .grid
            case .debloat: .feather
            }
        }
    }

    /// How often the safe findings are cleaned without asking.
    enum AutoClean: String, CaseIterable, Identifiable {
        case never, weekly, monthly

        var id: Self { self }

        var title: String {
            switch self {
            case .never: "Jamais"
            case .weekly: "Chaque semaine"
            case .monthly: "Chaque mois"
            }
        }

        var interval: TimeInterval? {
            switch self {
            case .never: nil
            case .weekly: 7 * 86_400
            case .monthly: 30 * 86_400
            }
        }
    }

    private enum Keys {
        static let autoClean = "autoClean"
        static let lastAutoClean = "lastAutoClean"
        static let watchUninstalls = "watchUninstalls"
        static let lowSpaceGB = "lowSpaceGB"
        static let welcomed = "welcomed"
    }

    var section: Section = .home
    /// Opens the main window, set by the menu bar icon (see `MenuBarLabel`).
    var windowOpener: (() -> Void)?
    private(set) var disk: DiskSpace?
    /// System memory figures, refreshed every minute (and by the memory view).
    private(set) var memoryStats: MemoryStatsInfo?
    /// Size of the Trash; `nil` until measured, or without Full Disk Access.
    private(set) var trashBytes: UInt64?
    /// While the Finder empties the Trash.
    private(set) var emptyingTrash = false

    let cleanup = CleanupModel()
    let explorer = ExplorerModel()
    let duplicates = DuplicatesModel()
    let photos = PhotosModel()
    let archive = ArchiveModel()
    let memory = MemoryModel()
    let apps = AppsModel()
    let debloat = DebloatModel()
    let updates = Updates()
    let history = DiskHistory()

    var autoClean: AutoClean {
        didSet { UserDefaults.standard.set(autoClean.rawValue, forKey: Keys.autoClean) }
    }

    /// Last cleanup run on its own, whatever it found.
    private(set) var lastAutoClean: Date? {
        didSet { UserDefaults.standard.set(lastAutoClean, forKey: Keys.lastAutoClean) }
    }

    /// When the next automatic cleanup is due, `nil` when they are off.
    var nextAutoClean: Date? {
        guard let interval = autoClean.interval else { return nil }
        return lastAutoClean.map { $0.addingTimeInterval(interval) } ?? .now
    }

    var watchUninstalls: Bool {
        didSet {
            UserDefaults.standard.set(watchUninstalls, forKey: Keys.watchUninstalls)
            watchUninstalls ? startWatching() : watcher.stop()
        }
    }

    /// The welcome sheet, shown until the user has seen it once.
    var showsWelcome: Bool {
        didSet { if !showsWelcome { UserDefaults.standard.set(true, forKey: Keys.welcomed) } }
    }

    /// Below this many gigabytes free, a notification is posted once.
    var lowSpaceGB: Int {
        didSet { UserDefaults.standard.set(lowSpaceGB, forKey: Keys.lowSpaceGB) }
    }

    private let notifier = Notifier()
    private let watcher = AppRemovalWatcher()
    private let scheduler = HourlyScheduler()
    private var started = false
    private var warnedLowSpace = false
    private var warnedMemory = false

    init() {
        let defaults = UserDefaults.standard
        watchUninstalls = defaults.object(forKey: Keys.watchUninstalls) as? Bool ?? true
        lowSpaceGB = defaults.object(forKey: Keys.lowSpaceGB) as? Int ?? 20
        showsWelcome = !defaults.bool(forKey: Keys.welcomed)
        autoClean = defaults.string(forKey: Keys.autoClean).flatMap(AutoClean.init) ?? .never
        lastAutoClean = defaults.object(forKey: Keys.lastAutoClean) as? Date
    }

    func start() async {
        guard !started else { return }
        started = true
        notifier.activate()
        notifier.onOpen = { [weak self] target in
            guard let self else { return }
            switch target {
            case .cleanup:
                showWindow(.cleanup)
            case .memory:
                showWindow(.memory)
            case .explorer:
                showWindow(.explorer)
            case let .uninstall(bundleId):
                showWindow(.apps)
                Task { await self.apps.prepareRemoved(bundleId: bundleId) }
            }
        }
        memory.onRefresh = { [weak self] stats in self?.memoryStats = stats }
        AppDelegate.onOpenApp = { [weak self] url in self?.uninstallDropped(url) }
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
        // la mesure du dossier perso enregistrée est rechargée tt de suite, Explorateur et Gros fichiers sont instantanés
        Task { [weak self] in await self?.explorer.restore() }
        scheduler.start { [weak self] in await self?.housekeeping() }
        // 1er passage un peu après le lancement pr pas ralentir l'ouverture
        Task { [weak self] in
            try? await Task.sleep(for: .seconds(30))
            await self?.housekeeping()
        }
    }

    /// What runs on its own about once an hour: today's disk measurement,
    /// with a word about folders that suddenly grew, and the automatic
    /// cleanup when it is due.
    func housekeeping() async {
        let grown = await history.recordIfDue()
        for growth in grown.prefix(2) {
            notifier.folderGrew(path: growth.path, grown: growth.grown)
        }
        await autoCleanIfDue()
    }

    private func autoCleanIfDue() async {
        guard let due = nextAutoClean, due <= .now else { return }
        // on touche à rien si l'utilisateur est déjà en train d'analyser ou de nettoyer
        switch cleanup.phase {
        case .scanning, .cleaning: return
        case .idle, .ready: break
        }
        await cleanup.scan()
        guard cleanup.phase == .ready else { return }
        lastAutoClean = .now
        // en dessous de 50 Mo ça vaut pas une notif
        guard cleanup.recommendedBytes >= 50_000_000 else { return }
        await cleanup.cleanRecommended()
        refreshDisk()
        if let outcome = cleanup.outcome {
            notifier.cleaned(bytes: outcome.freedBytes, failures: outcome.failures.count, automatic: true)
        }
    }

    /// Brings the window back, closed or not, on a page if one is given.
    func showWindow(_ section: Section? = nil) {
        if let section {
            self.section = section
        }
        windowOpener?()
    }

    /// An app dropped on the menu bar icon, its panel or the Dock icon:
    /// opens its uninstall review.
    func uninstallDropped(_ url: URL) {
        showWindow(.apps)
        Task { await apps.prepareDropped(url) }
    }

    /// The menu bar's one-click cleanup: the recommended findings only,
    /// reported by a notification since the window may be closed.
    func quickClean() async {
        await cleanup.cleanRecommended()
        refreshDisk()
        if let outcome = cleanup.outcome {
            notifier.cleaned(bytes: outcome.freedBytes, failures: outcome.failures.count)
        }
    }

    func refreshTrash() {
        Task {
            trashBytes = (try? await background { try trashSize() }) ?? nil
        }
    }

    /// Has the Finder empty the Trash; returns why it did not, if it did not.
    func emptyTrash() async -> String? {
        emptyingTrash = true
        let refusal = await SystemTrash.empty()
        emptyingTrash = false
        refreshTrash()
        refreshDisk()
        return refusal
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
            guard let stats = try? await background({ try LevraBindings.memoryStats() }) else { return }
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

    /// An app disappeared without Levra: tell the user what it left.
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
