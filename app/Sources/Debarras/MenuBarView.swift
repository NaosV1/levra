import AppKit
import SwiftUI

struct MenuBarView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        if let disk = model.disk {
            Text("Disque : \(Format.bytes(UInt64(max(disk.available, 0)))) libres sur \(Format.bytes(UInt64(max(disk.total, 0))))")
        }
        if let memory = model.memoryStats {
            Text("Mémoire : \(Format.bytes(memory.used)) sur \(Format.bytes(memory.total)) · pression \(memory.pressure.label.lowercased())")
        }
        Divider()
        Button("Voir ce qui occupe la mémoire") {
            model.section = .memory
            showWindow()
        }
        Button("Analyser maintenant") {
            model.section = .cleanup
            showWindow()
            Task { await model.cleanup.scan() }
        }
        Button("Désinstaller une app…") {
            model.section = .apps
            showWindow()
        }
        Button("Ouvrir Débarras") { showWindow() }
        Divider()
        SettingsLink { Text("Réglages…") }
        Button("Quitter Débarras") { NSApplication.shared.terminate(nil) }
            .keyboardShortcut("q")
    }

    private func showWindow() {
        openWindow(id: "main")
        NSApplication.shared.activate()
    }
}
