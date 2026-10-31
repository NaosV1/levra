import AppKit
import SwiftUI

/// The Finder icon of an app or file.
struct AppIcon: View {
    let path: String
    var size: CGFloat = 32

    var body: some View {
        Image(nsImage: NSWorkspace.shared.icon(forFile: path))
            .resizable()
            .frame(width: size, height: size)
            .accessibilityHidden(true)
    }
}

/// Panes of System Settings › Privacy & Security the app needs.
enum PrivacyPane: String {
    /// Reading the Trash, iPhone backups, Mail and Messages.
    case fullDiskAccess = "Privacy_AllFiles"
    /// Moving other apps' bundles to the Trash.
    case appManagement = "Privacy_AppBundles"
    /// Asking the Finder to trash what the app may not move itself.
    case automation = "Privacy_Automation"

    @MainActor
    func open() {
        guard let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?\(rawValue)") else { return }
        NSWorkspace.shared.open(url)
    }
}

/// System Settings › General › Login Items & Extensions, where the user
/// decides which apps may run in the background.
enum LoginItemsPane {
    @MainActor
    static func open() {
        guard let url = URL(string: "x-apple.systempreferences:com.apple.LoginItems-Settings.extension") else { return }
        NSWorkspace.shared.open(url)
    }
}

extension String {
    /// `/Users/me/Library` → `~/Library`.
    var abbreviatingHome: String {
        (self as NSString).abbreviatingWithTildeInPath
    }
}
