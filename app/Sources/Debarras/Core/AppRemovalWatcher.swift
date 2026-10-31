import DebarrasBindings
import Foundation

/// Notices apps leaving the application folders (dragged to the Trash,
/// deleted from Launchpad) and reports them with the identity recorded
/// while they were installed, so their leftovers can still be found.
///
/// It watches the folders themselves, which needs no special permission,
/// rather than the Trash, which requires Full Disk Access.
@MainActor
final class AppRemovalWatcher {
    /// An app updater replaces the bundle in two steps; waiting this long
    /// tells an update apart from a removal.
    private static let settleDelay: Duration = .seconds(2)

    var onRemoval: ((AppRecord) -> Void)?
    /// Called with the fresh app list after every change.
    var onListChanged: (([AppRecord]) -> Void)?

    private var known: [String: AppRecord] = [:]
    private var sources: [DispatchSourceFileSystemObject] = []
    private var pendingCheck: Task<Void, Never>?

    func start(folders: [URL], apps: [AppRecord]) {
        stop()
        remember(apps)
        for folder in folders {
            let descriptor = open(folder.path, O_EVTONLY)
            guard descriptor >= 0 else { continue }
            let source = DispatchSource.makeFileSystemObjectSource(
                fileDescriptor: descriptor,
                eventMask: [.write, .rename, .delete],
                queue: .main
            )
            source.setEventHandler { [weak self] in
                MainActor.assumeIsolated { self?.folderChanged() }
            }
            source.setCancelHandler { close(descriptor) }
            source.resume()
            sources.append(source)
        }
    }

    func stop() {
        sources.forEach { $0.cancel() }
        sources.removeAll()
        pendingCheck?.cancel()
    }

    func remember(_ apps: [AppRecord]) {
        known = Dictionary(apps.map { ($0.path, $0) }, uniquingKeysWith: { first, _ in first })
    }

    private func folderChanged() {
        pendingCheck?.cancel()
        pendingCheck = Task { [weak self] in
            try? await Task.sleep(for: Self.settleDelay)
            guard !Task.isCancelled else { return }
            await self?.checkForRemovals()
        }
    }

    private func checkForRemovals() async {
        guard let current = try? await background({ try listApps() }) else { return }
        let currentIds = Set(current.map(\.bundleId))
        // Gone from its path and not reinstalled elsewhere (an update or a
        // move between folders keeps the bundle id around).
        let removed = known.values.filter { app in
            !FileManager.default.fileExists(atPath: app.path) && !currentIds.contains(app.bundleId)
        }
        remember(current)
        onListChanged?(current)
        removed.forEach { onRemoval?($0) }
    }
}
