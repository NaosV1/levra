import AppKit
import SwiftUI

@main
struct DebarrasApp: App {
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
        Window("Débarras", id: "main") {
            RootView()
                .environment(model)
                .frame(minWidth: 900, minHeight: 560)
        }
        // The sidebar runs to the top; the window buttons sit on it.
        .windowStyle(.hiddenTitleBar)
        .defaultSize(width: 1120, height: 740)

        MenuBarExtra {
            MenuBarView().environment(model)
        } label: {
            Label(model.disk.map { Format.bytes(UInt64(max($0.available, 0))) } ?? "Débarras", systemImage: "internaldrive")
        }

        Settings {
            SettingsView().environment(model)
        }
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    /// Closing the window keeps the menu bar item and uninstall detection.
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        false
    }
}
