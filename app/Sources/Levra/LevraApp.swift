import AppKit
import SwiftUI

@main
struct LevraApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    @State private var model: AppModel

    init() {
        let model = AppModel()
        _model = State(initialValue: model)
        // Started here rather than from the window: at login the app may
        // run in the menu bar only, and detection must work anyway.
        Task { await model.start() }
    }

    var body: some Scene {
        Window("Levra", id: "main") {
            RootView()
                .environment(model)
                .frame(minWidth: 900, minHeight: 560)
        }
        // The sidebar runs to the top; the window buttons sit on it.
        .windowStyle(.hiddenTitleBar)
        .defaultSize(width: 1120, height: 740)
        .commands {
            CommandGroup(after: .appInfo) {
                Button("Rechercher les mises à jour…") { model.updates.check() }
                    .disabled(!model.updates.canCheck)
            }
        }

        MenuBarExtra {
            MenuBarView().environment(model)
        } label: {
            MenuBarLabel(model: model)
        }
        .menuBarExtraStyle(.window)

        Settings {
            SettingsView().environment(model)
        }
    }
}

/// The menu bar icon. Its view lives as long as the app, window closed or
/// not, which makes it the one place able to reopen the window for the
/// rest of the app: SwiftUI opens windows from views only.
private struct MenuBarLabel: View {
    let model: AppModel
    @Environment(\.openWindow) private var openWindow

    /// Levra's glyph as a template image, which the menu bar draws in its
    /// own colour, light or dark, like its other icons.
    @MainActor private static let icon: NSImage = {
        let renderer = ImageRenderer(content: LevraGlyph(size: 16))
        renderer.scale = 2
        let image = renderer.nsImage ?? NSImage()
        image.isTemplate = true
        return image
    }()

    var body: some View {
        Image(nsImage: Self.icon)
            .accessibilityLabel("Levra")
            .onAppear {
                model.windowOpener = {
                    openWindow(id: "main")
                    NSApplication.shared.activate()
                }
                StatusItemDrop.install { url in model.uninstallDropped(url) }
            }
    }
}

/// Levra is in the Dock only while one of its windows is open: once they
/// are closed, it lives on in the menu bar, uninstall detection included.
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    /// Apps dropped on the Dock icon, handed over once the model listens.
    static var onOpenApp: ((URL) -> Void)? {
        didSet {
            guard let onOpenApp else { return }
            pending.forEach(onOpenApp)
            pending = []
        }
    }

    private static var pending: [URL] = []

    // une app glissée sur l'icône du dock arrive ici, même au lancement avant que le modèle soit prêt
    func application(_ application: NSApplication, open urls: [URL]) {
        for url in urls where url.pathExtension == "app" {
            if let handler = Self.onOpenApp { handler(url) } else { Self.pending.append(url) }
        }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        false
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        let center = NotificationCenter.default
        center.addObserver(self, selector: #selector(windowBecameKey), name: NSWindow.didBecomeKeyNotification, object: nil)
        center.addObserver(self, selector: #selector(windowWillClose), name: NSWindow.willCloseNotification, object: nil)
    }

    @objc private func windowBecameKey(_ note: Notification) {
        guard let window = note.object as? NSWindow, Self.isAppWindow(window) else { return }
        NSApplication.shared.setActivationPolicy(.regular)
    }

    @objc private func windowWillClose(_ note: Notification) {
        guard let closing = note.object as? NSWindow, Self.isAppWindow(closing) else { return }
        let othersOpen = NSApplication.shared.windows.contains {
            $0 !== closing && $0.isVisible && Self.isAppWindow($0)
        }
        if !othersOpen {
            NSApplication.shared.setActivationPolicy(.accessory)
        }
    }

    /// The main window and Settings, as opposed to the menu bar panel,
    /// alerts and sheets, which are panels.
    private static func isAppWindow(_ window: NSWindow) -> Bool {
        window.styleMask.contains(.titled) && !(window is NSPanel)
    }
}
