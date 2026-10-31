import Foundation
import LevraBindings
import Observation

/// Development projects nobody opened for months: each can be compressed
/// (without its dependencies and builds, which come back on the next
/// install) into a folder of the user's choice, an external disk for
/// instance, before the original goes to the Trash.
@MainActor
@Observable
final class ArchiveModel {
    enum Phase: Equatable {
        case idle
        case scanning(files: UInt64, bytes: UInt64)
        case ready
        case archiving
    }

    private enum Keys {
        static let destination = "archiveDestination"
        static let idleMonths = "archiveIdleMonths"
    }

    private(set) var phase: Phase = .idle
    private(set) var projects: [OldProjectInfo] = []
    private(set) var error: String?
    private(set) var outcome: CleanupOutcome?
    /// Archives written by the last run, and their total size.
    private(set) var archivesCreated = 0
    private(set) var archivesBytes: UInt64 = 0
    var selection: Set<String> = []

    /// While archiving: the project in hand, those waiting, how many are done.
    private(set) var current: String?
    private(set) var queue: [String] = []
    private(set) var done = 0
    private(set) var total = 0

    var destination: URL {
        didSet { UserDefaults.standard.set(destination.path, forKey: Keys.destination) }
    }

    var idleMonths: Int {
        didSet { UserDefaults.standard.set(idleMonths, forKey: Keys.idleMonths) }
    }

    private var cancel: CancelHandle?
    private var stopRequested = false

    init() {
        let defaults = UserDefaults.standard
        destination = defaults.string(forKey: Keys.destination).map { URL(fileURLWithPath: $0) }
            ?? URL(fileURLWithPath: NSHomeDirectory()).appending(path: "Archives")
        idleMonths = defaults.object(forKey: Keys.idleMonths) as? Int ?? 6
    }

    var selectedProjects: [OldProjectInfo] {
        projects.filter { selection.contains($0.path) }
    }

    /// Roughly what archiving the selection frees: everything, minus what
    /// the archive keeps when it stays on the same disk.
    var estimatedFreed: UInt64 {
        selectedProjects.reduce(0) { total, project in
            // le code se compresse bien, on compte grossièrement un tiers de ce qui est gardé
            let kept = project.bytes - min(project.regenerableBytes, project.bytes)
            return total + (destinationIsOtherDisk ? project.bytes : project.bytes - kept / 3)
        }
    }

    /// Whether archives leave the startup disk, freeing all of it there.
    var destinationIsOtherDisk: Bool {
        let home = URL(fileURLWithPath: NSHomeDirectory())
        let key: Set<URLResourceKey> = [.volumeIdentifierKey]
        guard let target = try? existingAncestor(of: destination).resourceValues(forKeys: key).volumeIdentifier as? NSObject,
              let startup = try? home.resourceValues(forKeys: key).volumeIdentifier as? NSObject
        else { return false }
        return !target.isEqual(startup)
    }

    func find() async {
        let cancel = CancelHandle()
        self.cancel = cancel
        phase = .scanning(files: 0, bytes: 0)
        error = nil
        outcome = nil
        let relay = ProgressRelay { [weak self] files, bytes, _ in
            guard let self, case .scanning = self.phase else { return }
            self.phase = .scanning(files: files, bytes: bytes)
        }
        let days = UInt32(idleMonths * 30)
        do {
            projects = try await background { try findOldProjects(idleDays: days, cancel: cancel, listener: relay) }
            selection = selection.intersection(projects.map(\.path))
            phase = .ready
        } catch {
            self.error = error.message
            phase = .idle
        }
    }

    func cancelScan() {
        cancel?.cancel()
    }

    func archiveSelected() async {
        guard phase == .ready, !selection.isEmpty else { return }
        let paths = projects.map(\.path).filter(selection.contains)
        let destination = destination.path
        phase = .archiving
        stopRequested = false
        total = paths.count
        done = 0
        queue = paths
        archivesCreated = 0
        archivesBytes = 0

        var merged = ExecutionResult(completed: [], freedBytes: 0, needsAuthorization: [], failures: [])
        for path in paths {
            guard !stopRequested else { break }
            queue.removeFirst()
            current = path
            do {
                let result = try await background { try archiveProject(path: path, destination: destination) }
                archivesCreated += 1
                archivesBytes += result.archiveBytes
                merged.completed += result.result.completed
                merged.needsAuthorization += result.result.needsAuthorization
                merged.failures += result.result.failures
                // si l'archive reste sur le même disque elle reprend une partie de la place
                merged.freedBytes += result.result.freedBytes - min(result.result.freedBytes, destinationIsOtherDisk ? 0 : result.archiveBytes)
                if result.result.completed.contains(path) {
                    projects.removeAll { $0.path == path }
                    selection.remove(path)
                }
            } catch {
                merged.failures.append(FailureInfo(target: path, reason: error.message))
            }
            done += 1
        }
        current = nil
        queue = []
        let outcome = await CleanupOutcome.finishing(merged)
        self.outcome = outcome
        projects.removeAll { outcome.handled.contains($0.path) }
        selection.subtract(outcome.handled)
        phase = .ready
    }

    func stop() {
        stopRequested = true
        queue = []
    }

    func dismissOutcome() {
        outcome = nil
    }

    /// The destination may not exist yet: its volume is that of the
    /// closest folder that does.
    private func existingAncestor(of url: URL) -> URL {
        var url = url
        while !FileManager.default.fileExists(atPath: url.path), url.path != "/" {
            url.deleteLastPathComponent()
        }
        return url
    }
}
