import DebarrasBindings
import Foundation
import Observation

/// Summary shown after a cleanup or an uninstall.
struct CleanupOutcome: Equatable {
    var freedBytes: UInt64
    /// Items fully handled, including those the Finder moved.
    var completed: Int
    /// Of those, moved by the Finder with the user's authorization.
    var authorized: Int
    var failures: [FailureInfo]
    /// Identifiers of the items handled, plus the paths the Finder moved.
    var handled: Set<String>

    /// Hands what the core could not move to the Finder.
    @MainActor
    static func finishing(_ result: ExecutionResult) async -> CleanupOutcome {
        let pending = result.needsAuthorization
        let fallback = await SystemTrash.recycle(pending.map(\.path))
        let refused = Set(fallback.failed)
        // An item counts once every one of its paths is in the Trash.
        let unfinished = Set(pending.filter { refused.contains($0.path) }.map(\.id))
        let authorized = Set(pending.map(\.id)).subtracting(unfinished)
        return CleanupOutcome(
            freedBytes: result.freedBytes
                + pending.filter { authorized.contains($0.id) }.reduce(0) { $0 + $1.bytes },
            completed: result.completed.count + authorized.count,
            authorized: authorized.count,
            failures: result.failures + fallback.failed.map { FailureInfo(target: $0, reason: fallback.reason) },
            handled: Set(result.completed)
                .union(authorized)
                .union(pending.map(\.path).filter { !refused.contains($0) })
        )
    }
}

@MainActor
@Observable
final class CleanupModel {
    enum Phase: Equatable {
        case idle
        case scanning(files: UInt64, bytes: UInt64)
        case ready
        case cleaning
    }

    private(set) var phase: Phase = .idle
    private(set) var findings: [FindingInfo] = []
    private(set) var denied: [String] = []
    private(set) var outcome: CleanupOutcome?
    private(set) var error: String?
    var selection: Set<String> = []

    private var report: CleanupReport?
    private var cancel: CancelHandle?

    var selectedFindings: [FindingInfo] {
        findings.filter { selection.contains($0.id) }
    }

    var selectedBytes: UInt64 {
        selectedFindings.reduce(0) { $0 + $1.bytes }
    }

    var recommendedBytes: UInt64 {
        findings.filter(\.recommended).reduce(0) { $0 + $1.bytes }
    }

    /// Findings of each category, largest category first.
    var groups: [(category: DebarrasBindings.Category, findings: [FindingInfo], bytes: UInt64)] {
        DebarrasBindings.Category.ordered
            .compactMap { category in
                let items = findings.filter { $0.category == category }
                guard !items.isEmpty else { return nil }
                return (category, items, items.reduce(0) { $0 + $1.bytes })
            }
            .sorted { $0.bytes > $1.bytes }
    }

    func scan() async {
        guard phase != .cleaning else { return }
        let cancel = CancelHandle()
        self.cancel = cancel
        phase = .scanning(files: 0, bytes: 0)
        error = nil
        outcome = nil
        let relay = ProgressRelay { [weak self] files, bytes in
            guard let self, case .scanning = self.phase else { return }
            self.phase = .scanning(files: files, bytes: bytes)
        }
        do {
            let report = try await background { try scanCleanup(cancel: cancel, listener: relay) }
            self.report = report
            findings = report.findings()
            denied = report.deniedPaths()
            selection = Set(findings.filter(\.recommended).map(\.id))
            phase = .ready
        } catch {
            self.error = error.message
            phase = .idle
        }
    }

    func cancelScan() {
        cancel?.cancel()
    }

    func clean() async {
        guard let report, !selection.isEmpty else { return }
        phase = .cleaning
        let ids = Array(selection)
        let result = await background { report.execute(ids: ids) }
        outcome = await CleanupOutcome.finishing(result)
        let done = Set(result.completed)
        findings.removeAll { done.contains($0.id) }
        selection.subtract(done)
        phase = .ready
    }

    func dismissOutcome() {
        outcome = nil
    }
}
