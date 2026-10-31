import LevraBindings
import Foundation

/// Receives progress from the Rust reporter thread and hands it to the
/// main actor, where the models live.
final class ProgressRelay: ProgressListener {
    private let update: @MainActor @Sendable (UInt64, UInt64, UInt32) -> Void

    /// `update` receives the files and bytes read, and the bits of the
    /// steps over (for the cleanup scan).
    init(_ update: @escaping @MainActor @Sendable (UInt64, UInt64, UInt32) -> Void) {
        self.update = update
    }

    func onProgress(files: UInt64, bytes: UInt64, stepsDone: UInt32) {
        Task { @MainActor in update(files, bytes, stepsDone) }
    }
}

/// Runs a blocking call into the Rust core away from the main thread.
func background<T: Sendable>(_ work: @escaping @Sendable () throws -> T) async throws -> T {
    try await Task.detached(priority: .userInitiated, operation: work).value
}

func background<T: Sendable>(_ work: @escaping @Sendable () -> T) async -> T {
    await Task.detached(priority: .userInitiated, operation: work).value
}

extension Error {
    /// The French message of core errors, the system one otherwise.
    var message: String {
        (self as? LocalizedError)?.errorDescription ?? localizedDescription
    }
}
