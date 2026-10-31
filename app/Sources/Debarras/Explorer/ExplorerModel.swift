import DebarrasBindings
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
    private(set) var trail: [NodeInfo] = []
    private(set) var denied: [String] = []
    /// Items trashed since the scan: hidden, sizes above them are stale.
    private(set) var trashed: Set<UInt64> = []

    private var tree: ExploreTree?
    private var cancel: CancelHandle?

    var current: NodeInfo? { trail.last }

    var children: [NodeInfo] {
        guard let tree, let current else { return [] }
        return tree.children(id: current.id).filter { !trashed.contains($0.id) }
    }

    func choose(_ folder: URL) async {
        root = folder
        await scan()
    }

    func scan() async {
        let cancel = CancelHandle()
        self.cancel = cancel
        phase = .scanning(files: 0, bytes: 0)
        error = nil
        let relay = ProgressRelay { [weak self] files, bytes in
            guard let self, case .scanning = self.phase else { return }
            self.phase = .scanning(files: files, bytes: bytes)
        }
        let path = root.path
        do {
            let tree = try await background { try explore(path: path, cancel: cancel, listener: relay) }
            self.tree = tree
            trail = tree.root().map { [$0] } ?? []
            denied = tree.deniedPaths()
            trashed = []
            phase = .ready
        } catch {
            self.error = error.message
            phase = .idle
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
        tree?.path(id: node.id)
    }

    func largestFiles(minBytes: UInt64, untouchedDays: UInt32) -> [NodeInfo] {
        guard let tree else { return [] }
        return tree.largestFiles(limit: 200, minBytes: minBytes, untouchedDays: untouchedDays)
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
