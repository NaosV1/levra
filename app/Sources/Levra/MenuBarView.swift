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
        quickClean
        Divider()
        Button("Voir ce qui occupe la mémoire") {
            model.section = .memory
            showWindow()
        }
        Button("Désinstaller une app…") {
            model.section = .apps
            showWindow()
        }
        Button("Ouvrir Levra") { showWindow() }
        Divider()
        SettingsLink { Text("Réglages…") }
        Button("Quitter Levra") { NSApplication.shared.terminate(nil) }
            .keyboardShortcut("q")
    }

    /// Scans and cleans without opening the window.
    @ViewBuilder
    private var quickClean: some View {
        let cleanup = model.cleanup
        switch cleanup.phase {
        case .idle:
            Button("Chercher ce qui peut partir") { Task { await cleanup.scan() } }
        case .scanning:
            Text("Analyse en cours…")
        case .cleaning:
            Text("Nettoyage en cours…")
        case .ready:
            if cleanup.recommendedBytes > 0 {
                Button("Libérer \(Format.bytes(cleanup.recommendedBytes)) sans risque…") { confirmQuickClean() }
            } else {
                Text("Rien à libérer sans risque")
            }
            Button("Voir le détail") {
                model.section = .cleanup
                showWindow()
            }
            Button("Relancer l'analyse") { Task { await cleanup.scan() } }
        }
    }

    private func confirmQuickClean() {
        let alert = NSAlert()
        alert.messageText = "Libérer \(Format.bytes(model.cleanup.recommendedBytes)) sans risque ?"
        alert.informativeText = "Seuls les caches et les fichiers qui se recréent tout seuls sont retirés : la sélection recommandée du nettoyage."
        alert.addButton(withTitle: "Libérer")
        alert.addButton(withTitle: "Annuler")
        NSApplication.shared.activate()
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        Task { await model.quickClean() }
    }

    private func showWindow() {
        openWindow(id: "main")
        NSApplication.shared.activate()
    }
}
