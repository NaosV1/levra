import CoreServices
import Foundation

/// What changed on disk while Levra was not looking, from the history
/// macOS keeps of every folder modified (FSEvents), app closed or not.
enum ChangeHistory {
    struct Changes: Sendable {
        /// Folders whose content changed.
        let folders: [String]
        /// The history is incomplete: only a full measurement is right.
        let mustRescan: Bool
        /// Event id the changes go up to, to save with the measurement.
        let latest: UInt64
    }

    /// Past this many folders, measuring everything again is simpler.
    private static let maxFolders = 20_000

    static var currentEventId: UInt64 {
        FSEventsGetCurrentEventId()
    }

    /// Identifies the history of the volume holding `path`: it starts over
    /// (and old event ids mean nothing) when this changes.
    static func historyId(for path: String) -> String? {
        var info = stat()
        guard stat(path, &info) == 0, let uuid = FSEventsCopyUUIDForDevice(info.st_dev) else { return nil }
        return CFUUIDCreateString(nil, uuid) as String?
    }

    /// Folders changed under `root` since the event `since`.
    static func changes(under root: String, since: UInt64) async -> Changes {
        let collector = Collector(limit: maxFolders)
        return await withCheckedContinuation { continuation in
            collector.onDone = { continuation.resume(returning: $0) }
            var context = FSEventStreamContext(
                version: 0,
                info: Unmanaged.passRetained(collector).toOpaque(),
                retain: nil,
                release: { info in
                    if let info { Unmanaged<Collector>.fromOpaque(info).release() }
                },
                copyDescription: nil
            )
            let flags = UInt32(kFSEventStreamCreateFlagUseCFTypes | kFSEventStreamCreateFlagNoDefer)
            guard let stream = FSEventStreamCreate(
                nil,
                { _, info, count, paths, flags, ids in
                    guard let info else { return }
                    let collector = Unmanaged<Collector>.fromOpaque(info).takeUnretainedValue()
                    let paths = unsafeBitCast(paths, to: NSArray.self) as? [String] ?? []
                    for index in 0..<count {
                        collector.add(path: index < paths.count ? paths[index] : nil, flags: flags[index], id: ids[index])
                    }
                },
                &context,
                [root] as CFArray,
                since,
                0.1,
                flags
            ) else {
                collector.finish(rescan: true)
                return
            }
            collector.stream = stream
            FSEventStreamSetDispatchQueue(stream, DispatchQueue(label: "com.levra.app.history"))
            FSEventStreamStart(stream)
            // si macOS ne dit jamais "historique fini" on lâche l'affaire et on remesure tout
            DispatchQueue.global().asyncAfter(deadline: .now() + 15) { collector.finish(rescan: true) }
        }
    }
}

/// Gathers the replayed events until macOS says the history is over.
private final class Collector: @unchecked Sendable {
    var onDone: ((ChangeHistory.Changes) -> Void)?
    var stream: FSEventStreamRef?

    private let lock = NSLock()
    private let limit: Int
    private var folders: Set<String> = []
    private var rescan = false
    private var latest: UInt64 = 0
    private var finished = false

    private static let incomplete = UInt32(
        kFSEventStreamEventFlagMustScanSubDirs | kFSEventStreamEventFlagUserDropped
            | kFSEventStreamEventFlagKernelDropped | kFSEventStreamEventFlagEventIdsWrapped
            | kFSEventStreamEventFlagRootChanged
    )

    init(limit: Int) {
        self.limit = limit
    }

    func add(path: String?, flags: FSEventStreamEventFlags, id: FSEventStreamEventId) {
        if flags & UInt32(kFSEventStreamEventFlagHistoryDone) != 0 {
            finish(rescan: false)
            return
        }
        lock.withLock {
            latest = max(latest, id)
            if flags & Self.incomplete != 0 { rescan = true }
            if let path {
                // macOS met parfois un / à la fin, l'arbre lui n'en veut pas
                folders.insert(path.count > 1 && path.hasSuffix("/") ? String(path.dropLast()) : path)
            }
            if folders.count > limit { rescan = true }
        }
    }

    func finish(rescan forced: Bool) {
        let result: ChangeHistory.Changes? = lock.withLock {
            guard !finished else { return nil }
            finished = true
            return ChangeHistory.Changes(
                folders: Array(folders),
                mustRescan: rescan || forced,
                latest: max(latest, ChangeHistory.currentEventId)
            )
        }
        guard let result else { return }
        if let stream {
            FSEventStreamStop(stream)
            FSEventStreamInvalidate(stream)
            FSEventStreamRelease(stream)
        }
        onDone?(result)
    }
}
