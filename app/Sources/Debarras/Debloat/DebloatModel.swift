import DebarrasBindings
import Observation

/// Agents and services installed by apps, and Apple's apps.
@MainActor
@Observable
final class DebloatModel {
    enum Phase: Equatable {
        case idle
        case loading
        case ready
        case removing
    }

    private(set) var phase: Phase = .idle
    private(set) var session: DebloatSession?
    private(set) var items: [BackgroundItemInfo] = []
    private(set) var systemAppCount: UInt64 = 0
    private(set) var systemAppBytes: UInt64 = 0
    private(set) var appleApps: [AppRecord] = []
    private(set) var outcome: CleanupOutcome?
    private(set) var error: String?
    var selection: Set<String> = []

    /// Left by apps that are gone: what Débarras may remove.
    var strayItems: [BackgroundItemInfo] { items.filter(\.removable) }
    /// Managed by an installed app or by Homebrew.
    var ownedItems: [BackgroundItemInfo] { items.filter { !$0.removable } }
    var loadedCount: Int { items.filter { $0.loaded == true }.count }

    /// `installed` is the list the Applications page shows: each job is
    /// attributed to those apps.
    func load(installed: [AppRecord]) async {
        phase = .loading
        error = nil
        do {
            let session = try await background { try scanDebloat(installed: installed) }
            self.session = session
            items = session.backgroundItems()
            systemAppCount = session.systemAppCount()
            systemAppBytes = session.systemAppBytes()
            appleApps = session.removableAppleApps()
            selection = Set(items.filter(\.selected).map(\.id))
            phase = .ready
        } catch {
            self.error = error.message
            phase = .idle
        }
    }

    func selectProposed() {
        selection = Set(items.filter(\.selected).map(\.id))
    }

    func setSelected(_ selected: Bool, _ item: BackgroundItemInfo) {
        guard item.removable else { return }
        if selected { selection.insert(item.id) } else { selection.remove(item.id) }
    }

    func remove(installed: [AppRecord]) async {
        guard let session, !selection.isEmpty else { return }
        phase = .removing
        let ids = Array(selection)
        let result = await background { session.execute(ids: ids) }
        outcome = await CleanupOutcome.finishing(result)
        await load(installed: installed)
    }

    func dismissOutcome() {
        outcome = nil
    }
}
