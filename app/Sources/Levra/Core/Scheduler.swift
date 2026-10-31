import Foundation

/// Wakes the app about once an hour, menu bar only or not, at a moment
/// macOS finds convenient: on power, with the Mac not busy.
final class HourlyScheduler: @unchecked Sendable {
    private let scheduler = NSBackgroundActivityScheduler(identifier: "com.levra.app.housekeeping")

    /// The completion handler of one wake-up, carried to the main actor.
    private struct Completion: @unchecked Sendable {
        let finish: NSBackgroundActivityScheduler.CompletionHandler
    }

    func start(_ work: @escaping @MainActor @Sendable () async -> Void) {
        scheduler.repeats = true
        scheduler.interval = 3600
        scheduler.tolerance = 600
        scheduler.qualityOfService = .utility
        scheduler.schedule { finish in
            let completion = Completion(finish: finish)
            // macOS choisit le moment, nous on fait juste le ménage qd il nous réveille
            Task { @MainActor in
                await work()
                completion.finish(.finished)
            }
        }
    }

    func stop() {
        scheduler.invalidate()
    }
}
