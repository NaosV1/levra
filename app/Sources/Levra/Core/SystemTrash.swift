import AppKit

/// Items the core could not move (owned by root, app bundles under App
/// Management protection, other apps' containers) are handed to the
/// Finder, which asks for the administrator password when it needs to, as
/// when the user drags them to the Trash. `NSWorkspace.recycle` cannot be
/// used for this: it fails on them without asking.
enum SystemTrash {
    struct Outcome {
        var moved = 0
        var failed: [String] = []
        /// Why the Finder did not move the failed paths.
        var reason = "macOS a refusé l'accès (autorisation non accordée)"
    }

    /// One request for every path, so that the password is asked once.
    @MainActor
    static func recycle(_ paths: [String]) async -> Outcome {
        guard !paths.isEmpty else { return Outcome() }
        letFinderAsk()
        let refusal = await Task.detached(priority: .userInitiated) { askFinderToTrash(paths) }.value
        // The Finder may move some items and not others: what is still
        // there was not moved, whatever the reply says.
        let failed = paths.filter { (try? FileManager.default.attributesOfItem(atPath: $0)) != nil }
        var outcome = Outcome(moved: paths.count - failed.count, failed: failed)
        if let refusal {
            outcome.reason = refusal
        }
        return outcome
    }

    /// Has the Finder empty the Trash, for good. The Finder asks for the
    /// password for items that need it, and needs no access from Levra.
    /// Returns why it refused, or `nil`.
    @MainActor
    static func empty() async -> String? {
        letFinderAsk()
        return await Task.detached(priority: .userInitiated) {
            let finder = NSAppleEventDescriptor(bundleIdentifier: "com.apple.finder")
            let event = NSAppleEventDescriptor(
                eventClass: fourCharCode("fndr"),
                eventID: fourCharCode("empt"),
                targetDescriptor: finder,
                returnID: AEReturnID(kAutoGenerateReturnID),
                transactionID: AETransactionID(kAnyTransactionID)
            )
            return send(event)
        }.value
    }

    /// Shows the Trash in the Finder, which reads it: Levra needs no
    /// access to it for that.
    @MainActor
    static func open() {
        NSWorkspace.shared.open(URL(fileURLWithPath: NSHomeDirectory()).appending(path: ".Trash"))
    }

    @MainActor
    static func reveal(_ path: String) {
        NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: path)])
    }

    /// Sends the Finder its "delete" event with the files as a list of file
    /// URLs: no script source is built, so no file name can be read as
    /// code. Returns why the Finder refused, or `nil`.
    private static func askFinderToTrash(_ paths: [String]) -> String? {
        let finder = NSAppleEventDescriptor(bundleIdentifier: "com.apple.finder")
        let event = NSAppleEventDescriptor(
            eventClass: AEEventClass(kAECoreSuite),
            eventID: AEEventID(kAEDelete),
            targetDescriptor: finder,
            returnID: AEReturnID(kAutoGenerateReturnID),
            transactionID: AETransactionID(kAnyTransactionID)
        )
        let files = NSAppleEventDescriptor.list()
        for path in paths {
            // Index 0 appends.
            files.insert(NSAppleEventDescriptor(fileURL: URL(fileURLWithPath: path)), at: 0)
        }
        event.setParam(files, forKeyword: keyDirectObject)
        return send(event)
    }

    /// The Finder may ask for the password only on behalf of the app in
    /// front; from the menu bar panel, Levra is not, and the Finder gave up
    /// without asking. Brought forward, Levra lets the Finder come forward
    /// in turn (`canSwitchLayer`) when it has something to ask.
    @MainActor
    private static func letFinderAsk() {
        NSApplication.shared.activate()
    }

    /// Sends an event to the Finder and waits, long enough for the user to
    /// type the password. Returns why the Finder refused, or `nil`.
    private static func send(_ event: NSAppleEventDescriptor) -> String? {
        do {
            let reply = try event.sendEvent(options: [.waitForReply, .canInteract, .canSwitchLayer], timeout: 600)
            guard let code = reply.paramDescriptor(forKeyword: keyErrorNumber)?.int32Value, code != 0 else {
                return nil
            }
            return refusal(code)
        } catch {
            return refusal(Int32(truncatingIfNeeded: (error as NSError).code))
        }
    }

    /// The Finder's suite and "empty trash" event come from FinderRegistry.h,
    /// which Swift does not import: built from their four letters here.
    private static func fourCharCode(_ code: String) -> FourCharCode {
        code.utf8.reduce(0) { $0 << 8 | FourCharCode($1) }
    }

    private static func refusal(_ code: Int32) -> String {
        switch code {
        case -128: "Le mot de passe n'a pas été saisi."
        case -1743: "Levra n'a pas le droit de piloter le Finder : autorise-le dans Réglages Système › Confidentialité et sécurité › Automatisation."
        case -1712: "Le Finder n'a pas répondu à temps."
        default: "Le Finder a refusé (erreur \(code))."
        }
    }
}
