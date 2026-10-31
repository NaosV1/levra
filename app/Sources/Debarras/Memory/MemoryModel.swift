import AppKit
import DebarrasBindings
import Observation

@MainActor
@Observable
final class MemoryModel {
    private(set) var snapshot: MemorySnapshotInfo?
    private(set) var error: String?
    /// Why the last quit did not fully happen.
    private(set) var quitError: String?
    /// Lets the sidebar show the same figures as the live view.
    var onRefresh: ((MemoryStatsInfo) -> Void)?

    /// Refreshes every two seconds until the calling task is cancelled
    /// (the memory view's `.task`, when the view goes away).
    func runLiveUpdates() async {
        while !Task.isCancelled {
            await refresh()
            try? await Task.sleep(for: .seconds(2))
        }
    }

    func refresh() async {
        do {
            let snapshot = try await background { try memorySnapshot() }
            self.snapshot = snapshot
            error = nil
            onRefresh?(snapshot.stats)
        } catch {
            self.error = error.message
        }
    }

    /// Apps are asked to quit through AppKit, so they can offer to save;
    /// command-line programs get a signal from the core, which checks again
    /// that each process is still the one listed.
    func quit(_ group: ProcessGroupInfo, force: Bool) async {
        quitError = nil
        switch group.kind {
        case .app:
            let running = group.bundleId.map(NSRunningApplication.runningApplications(withBundleIdentifier:)) ?? []
            if running.isEmpty {
                quitError = "\(group.name) n'a pas de fenêtre ouverte : ce sont des processus d'arrière-plan qu'elle relance elle-même."
            }
            for app in running {
                _ = force ? app.forceTerminate() : app.terminate()
            }
        case .tool:
            for process in group.processes {
                let (pid, path) = (process.pid, process.path)
                do {
                    try await background { try terminateProcess(pid: pid, path: path, force: force) }
                } catch {
                    quitError = error.message
                }
            }
        case .virtualMachine, .system:
            return
        }
        try? await Task.sleep(for: .seconds(1))
        await refresh()
    }

    func dismissQuitError() {
        quitError = nil
    }
}
