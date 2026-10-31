import DebarrasBindings
import Foundation
import Observation

@MainActor
@Observable
final class DuplicatesModel {
    enum Phase: Equatable {
        case idle
        case scanning(files: UInt64, bytes: UInt64)
        case ready
        case removing
    }

    private(set) var phase: Phase = .idle
    private(set) var groups: [DuplicateGroupInfo] = []
    private(set) var cloneGroups: UInt64 = 0
    private(set) var denied: [String] = []
    private(set) var outcome: CleanupOutcome?
    private(set) var error: String?
    /// Paths of the copies to move to the Trash.
    private(set) var selection: Set<String> = []

    private var scan: DuplicateScan?
    private var cancel: CancelHandle?

    var selectedBytes: UInt64 {
        groups.flatMap(\.files).filter { selection.contains($0.path) }.reduce(0) { $0 + $1.reclaimable }
    }

    var reclaimableBytes: UInt64 {
        groups.reduce(0) { $0 + $1.reclaimable }
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
        do {
            let scan = try await background { try findDuplicates(cancel: cancel, listener: relay) }
            self.scan = scan
            groups = scan.groups()
            cloneGroups = scan.cloneGroups()
            denied = scan.deniedPaths()
            selectProposed()
            phase = .ready
        } catch {
            self.error = error.message
            phase = .idle
        }
    }

    func cancelScan() {
        cancel?.cancel()
    }

    /// Every copy except the one proposed to keep.
    func selectProposed() {
        selection = Set(groups.flatMap(\.files).filter { !$0.keep }.map(\.path))
    }

    /// Selecting is refused when it would leave no copy of the group.
    /// (The core refuses it too; this only keeps the checkboxes honest.)
    func setSelected(_ selected: Bool, file: DuplicateFileInfo, in group: DuplicateGroupInfo) {
        guard selected else {
            selection.remove(file.path)
            return
        }
        let others = group.files.filter { $0.path != file.path }
        guard others.contains(where: { !selection.contains($0.path) }) else { return }
        selection.insert(file.path)
    }

    func remove() async {
        guard let scan, !selection.isEmpty else { return }
        phase = .removing
        let paths = Array(selection)
        let result = await background { scan.execute(paths: paths) }
        let outcome = await CleanupOutcome.finishing(result)
        self.outcome = outcome
        groups = groups.compactMap { group in
            var group = group
            group.files.removeAll { outcome.handled.contains($0.path) }
            guard group.files.count >= 2 else { return nil }
            // Files come oldest first: if the proposed copy went, the next one takes its place.
            if !group.files.contains(where: \.keep) {
                group.files[0].keep = true
            }
            group.reclaimable = group.files.filter { !$0.keep }.reduce(0) { $0 + $1.reclaimable }
            return group
        }
        selection.subtract(outcome.handled)
        phase = .ready
    }

    func dismissOutcome() {
        outcome = nil
    }
}
