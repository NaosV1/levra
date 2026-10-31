import LevraBindings
import Foundation
import Observation

/// One scan of a folder, browsed level by level; also feeds the large
/// files view.
@MainActor
@Observable
final class ExplorerModel {
    enum Phase: Equatable {
        case idle
        case scanning(files: UInt64, bytes: UInt64)
        case ready
    }

    private(set) var root = URL(fileURLWithPath: NSHomeDirectory())
    private(set) var phase: Phase = .idle
    private(set) var error: String?
    /// From the scanned folder down to the one displayed.
    private(set) var trail: [NodeInfo] = [] {
        didSet { refreshChildren() }
    }
    private(set) var denied: [String] = []
    /// Items trashed since the scan: hidden, sizes above them are stale.
    private(set) var trashed: Set<UInt64> = [] {
        didSet { refreshChildren() }
    }
    /// Contents of the folder displayed, without what went to the Trash.
    // gardé en cache sinon chq rendu repassait par le rust pr tt les lignes
    private(set) var children: [NodeInfo] = []

    private var tree: ExploreTree?
    private var cancel: CancelHandle?

    /// What macOS itself keeps at the top of the startup disk. Measuring the
    /// whole disk shows them grouped, out of the way of the user's folders.
    static let systemFolders: Set<String> = [
        "System", "private", "usr", "bin", "sbin", "dev", "cores", "Volumes",
        "etc", "var", "tmp", "home", ".vol", ".resolve", ".file", ".nofollow",
    ]

    var current: NodeInfo? { trail.last }

    /// The measured folder in words: "ton dossier personnel" rather than "~".
    var rootName: String {
        root.path == NSHomeDirectory() ? "ton dossier personnel" : root.path.abbreviatingHome
    }

    private func refreshChildren() {
        guard let tree, let current else {
            children = []
            return
        }
        children = tree.children(id: current.id).filter { !trashed.contains($0.id) }
    }

    /// Whether `node` is one of macOS's own folders at the root of the disk.
    func isSystemFolder(_ node: NodeInfo) -> Bool {
        root.path == "/" && trail.count == 1 && Self.systemFolders.contains(node.name)
    }

    func choose(_ folder: URL) async {
        root = folder
        await scan()
    }

    /// When the measurement shown was taken, set when it comes from the
    /// saved one and is being brought up to date.
    private(set) var measuredAt: Date?
    /// The saved measurement is shown while changes are being read.
    private(set) var updating = false

    private static let historyKey = "explorerHistoryId"

    /// Saved measurement of the home folder, the only one kept.
    private static var cacheFile: URL {
        FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
            .appending(path: "com.levra.app", directoryHint: .isDirectory)
            .appending(path: "explorer-home.bin")
    }

    private var isHome: Bool { root.path == NSHomeDirectory() }

    /// What the pages call when they open: the saved measurement at once,
    /// brought up to date in the background, or a fresh one.
    func openDefault() async {
        guard phase == .idle, error == nil else { return }
        if await !restore() {
            await scan()
        }
    }

    /// Shows the saved measurement of the home folder, then reads what
    /// changed since (from macOS's history of changes) and updates only
    /// those folders. Returns `false` when there is nothing to show.
    @discardableResult
    func restore() async -> Bool {
        guard phase == .idle, isHome else { return false }
        let file = Self.cacheFile.path
        let home = root.path
        // le disque a changé d'historique (réinstall, autre volume) : les numéros d'évènements valent plus rien
        guard let historyId = ChangeHistory.historyId(for: home),
              UserDefaults.standard.string(forKey: Self.historyKey) == historyId,
              let cached = try? await background({ try loadExploreCache(path: file) }),
              cached.tree.rootPath() == home
        else { return false }

        apply(cached.tree, sameIds: false)
        measuredAt = (try? FileManager.default.attributesOfItem(atPath: file)[.modificationDate]) as? Date
        phase = .ready
        updating = true
        defer { updating = false }

        let changes = await ChangeHistory.changes(under: home, since: cached.stamp)
        guard phase == .ready, root.path == home else { return true }
        if changes.mustRescan {
            await scan(keepingCurrent: true)
            return true
        }
        if !changes.folders.isEmpty {
            let cancel = CancelHandle()
            self.cancel = cancel
            let tree = cached.tree
            let folders = changes.folders
            let relay = ProgressRelay { _, _, _ in }
            guard let refreshed = try? await background({ try tree.refreshed(changed: folders, cancel: cancel, listener: relay) }) else {
                await scan(keepingCurrent: true)
                return true
            }
            apply(refreshed, sameIds: true)
        }
        measuredAt = nil
        save(stamp: changes.latest)
        return true
    }

    /// Measures the folder again. With `keepingCurrent`, the current
    /// measurement stays on screen meanwhile.
    func scan(keepingCurrent: Bool = false) async {
        let cancel = CancelHandle()
        self.cancel = cancel
        let keep = keepingCurrent && tree != nil
        if keep {
            updating = true
        } else {
            phase = .scanning(files: 0, bytes: 0)
        }
        error = nil
        let relay = ProgressRelay { [weak self] files, bytes, _ in
            guard let self, case .scanning = self.phase else { return }
            self.phase = .scanning(files: files, bytes: bytes)
        }
        let path = root.path
        // on note où en est l'historique AVANT de mesurer, ce qui bouge pendant sera rattrapé au prochain lancement
        let stamp = ChangeHistory.currentEventId
        do {
            let tree = try await background { try explore(path: path, cancel: cancel, listener: relay) }
            apply(tree, sameIds: false)
            measuredAt = nil
            phase = .ready
            save(stamp: stamp)
        } catch {
            self.error = error.message
            if !keep { phase = .idle }
        }
        if keep { updating = false }
    }

    /// Shows another measurement of the same folder and finds the folders
    /// on display again in it.
    private func apply(_ tree: ExploreTree, sameIds: Bool) {
        let paths = trail.map(\.path)
        self.tree = tree
        if !sameIds { trashed = [] }
        denied = tree.deniedPaths()
        var trail: [NodeInfo] = []
        if let root = tree.root() {
            trail.append(root)
            for path in paths.dropFirst() {
                guard let node = tree.nodeAt(path: path) else { break }
                trail.append(node)
            }
        }
        self.trail = trail
    }

    private func save(stamp: UInt64) {
        guard isHome, let tree else { return }
        let file = Self.cacheFile
        let historyId = ChangeHistory.historyId(for: root.path)
        Task.detached(priority: .utility) {
            try? FileManager.default.createDirectory(at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
            guard (try? tree.saveCache(path: file.path, stamp: stamp)) != nil else { return }
            await MainActor.run { UserDefaults.standard.set(historyId, forKey: Self.historyKey) }
        }
    }

    func cancelScan() {
        cancel?.cancel()
    }

    func open(_ node: NodeInfo) {
        guard node.kind == .directory, node.hasChildren else { return }
        trail.append(node)
    }

    func goBack(to index: Int) {
        guard trail.indices.contains(index) else { return }
        trail.removeSubrange((index + 1)...)
    }

    func path(of node: NodeInfo) -> String? {
        node.path
    }

    func largestFiles(minBytes: UInt64, untouchedDays: UInt32, limit: UInt32 = 200) -> [NodeInfo] {
        guard let tree else { return [] }
        return tree.largestFiles(limit: limit, minBytes: minBytes, untouchedDays: untouchedDays)
            .filter { !trashed.contains($0.id) }
    }

    func trash(_ nodes: [NodeInfo]) async -> CleanupOutcome? {
        let paths = nodes.compactMap { path(of: $0) }
        guard !paths.isEmpty else { return nil }
        let result: ExecutionResult
        do {
            result = try await background { try moveToTrash(paths: paths) }
        } catch {
            self.error = error.message
            return nil
        }
        let outcome = await CleanupOutcome.finishing(result)
        for node in nodes where path(of: node).map(outcome.handled.contains) == true {
            trashed.insert(node.id)
        }
        return outcome
    }
}
