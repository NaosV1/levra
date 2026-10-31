import Combine
import Foundation
import Observation
import Sparkle

/// Updates, through Sparkle: once a day, Levra reads the appcast of the
/// latest GitHub release (`SUFeedURL` in Info.plist) and installs a newer
/// version only if it is signed with Levra's key (`SUPublicEDKey`), on top
/// of Apple's notarization.
@MainActor
@Observable
final class Updates {
    /// False while a check or an installation is under way.
    private(set) var canCheck = false

    var checksAutomatically: Bool {
        didSet { controller.updater.automaticallyChecksForUpdates = checksAutomatically }
    }

    /// "0.2.0 (148)": the version, then the build number Sparkle compares.
    let version: String

    @ObservationIgnored private let controller: SPUStandardUpdaterController
    @ObservationIgnored private var subscription: AnyCancellable?

    init() {
        controller = SPUStandardUpdaterController(startingUpdater: true, updaterDelegate: nil, userDriverDelegate: nil)
        checksAutomatically = controller.updater.automaticallyChecksForUpdates
        let info = Bundle.main.infoDictionary ?? [:]
        version = "\(info["CFBundleShortVersionString"] as? String ?? "?") (\(info["CFBundleVersion"] as? String ?? "?"))"
        subscription = controller.updater.publisher(for: \.canCheckForUpdates)
            .receive(on: DispatchQueue.main)
            .sink { [weak self] canCheck in
                MainActor.assumeIsolated { self?.canCheck = canCheck }
            }
    }

    /// Checks now, and says so even when there is nothing new.
    func check() {
        controller.checkForUpdates(nil)
    }
}
