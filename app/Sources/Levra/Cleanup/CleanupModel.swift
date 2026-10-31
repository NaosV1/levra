import LevraBindings
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
        /// `steps` has bit `n` set once `CleanupModel.steps[n]` is over.
        case scanning(files: UInt64, bytes: UInt64, steps: UInt32)
        case ready
        case cleaning
    }

    /// What the scan looks at, in the order of the progress bits.
    static let steps = scanSteps()

    private(set) var phase: Phase = .idle
    private(set) var findings: [FindingInfo] = []
    private(set) var denied: [String] = []
    private(set) var outcome: CleanupOutcome?
    private(set) var error: String?
    var selection: Set<String> = []

    /// While cleaning: the item being removed, those waiting behind it,
    /// how many are over and what they freed so far.
    private(set) var cleaningCurrent: String?
    private(set) var cleaningQueue: [String] = []
    private(set) var cleaningTotal = 0
    private(set) var cleaningDone = 0
    private(set) var freedSoFar: UInt64 = 0

    private var report: CleanupReport?
    private var cancel: CancelHandle?
    private var stopRequested = false

    var selectedFindings: [FindingInfo] {
        findings.filter { selection.contains($0.id) }
    }

    var selectedBytes: UInt64 {
        selectedFindings.reduce(0) { $0 + $1.bytes }
    }

    var recommendedBytes: UInt64 {
        findings.filter(\.recommended).reduce(0) { $0 + $1.bytes }
    }

    /// How much thought a finding needs before it goes: the page sorts
    /// findings this way, from the safest to the most delicate.
    enum Decision: CaseIterable {
        /// Regenerates on its own, and nothing uses it right now.
        case safe
        /// Regenerates too, but a project used it lately.
        case recentlyUsed
        case review
        case careful

        init(_ finding: FindingInfo) {
            switch finding.safety {
            case .safe: self = finding.recommended ? .safe : .recentlyUsed
            case .review: self = .review
            case .careful: self = .careful
            }
        }
    }

    /// Findings by decision, each section largest first.
    var sections: [(decision: Decision, findings: [FindingInfo], bytes: UInt64)] {
        Decision.allCases.compactMap { decision in
            let items = findings.filter { Decision($0) == decision }
            guard !items.isEmpty else { return nil }
            return (decision, items, items.reduce(0) { $0 + $1.bytes })
        }
    }

    func scan() async {
        guard phase != .cleaning else { return }
        let cancel = CancelHandle()
        self.cancel = cancel
        phase = .scanning(files: 0, bytes: 0, steps: 0)
        error = nil
        outcome = nil
        let relay = ProgressRelay { [weak self] files, bytes, steps in
            guard let self, case .scanning = self.phase else { return }
            self.phase = .scanning(files: files, bytes: bytes, steps: steps)
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

    /// Cleans the recommended findings only, whatever was selected.
    func cleanRecommended() async {
        selection = Set(findings.filter(\.recommended).map(\.id))
        await clean()
    }

    func clean() async {
        guard let report, !selection.isEmpty, phase != .cleaning else { return }
        // un par un du plus gros au plus petit, comme ça on montre où on en est pour de vrai
        let ids = findings.map(\.id).filter(selection.contains)
        phase = .cleaning
        stopRequested = false
        cleaningTotal = ids.count
        cleaningDone = 0
        freedSoFar = 0
        cleaningQueue = ids

        var completed: [String] = []
        var pending: [PendingRemoval] = []
        var failures: [FailureInfo] = []
        var freed: UInt64 = 0
        for id in ids {
            guard !stopRequested else { break }
            cleaningQueue.removeFirst()
            cleaningCurrent = id
            let result = await background { report.execute(ids: [id]) }
            completed += result.completed
            pending += result.needsAuthorization
            failures += result.failures
            freed += result.freedBytes
            freedSoFar = freed
            cleaningDone += 1
            // la ligne part dès que c fait, pas tt d'un coup à la fin
            if result.completed.contains(id) {
                findings.removeAll { $0.id == id }
                selection.remove(id)
            }
        }
        cleaningCurrent = nil
        cleaningQueue = []

        let result = ExecutionResult(
            completed: completed,
            freedBytes: freed,
            needsAuthorization: pending,
            failures: failures
        )
        let outcome = await CleanupOutcome.finishing(result)
        self.outcome = outcome
        findings.removeAll { outcome.handled.contains($0.id) }
        selection.subtract(outcome.handled)
        phase = .ready
    }

    /// Stops after the item being removed: one cannot be cut in half.
    func stopCleaning() {
        stopRequested = true
        cleaningQueue = []
    }

    func dismissOutcome() {
        outcome = nil
    }
}
