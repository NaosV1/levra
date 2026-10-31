import Foundation
import UserNotifications

/// Local notifications: low disk space, critical memory pressure, and apps
/// removed outside Levra whose leftovers are still on disk.
final class Notifier: NSObject, UNUserNotificationCenterDelegate, @unchecked Sendable {
    /// What clicking a notification opens.
    enum Target: Sendable {
        case cleanup
        case memory
        case explorer
        case uninstall(bundleId: String)
    }

    private enum Key {
        static let target = "target"
        static let bundleId = "bundleId"
    }

    @MainActor var onOpen: ((Target) -> Void)?

    private var center: UNUserNotificationCenter { .current() }

    func activate() {
        center.delegate = self
        center.requestAuthorization(options: [.alert, .sound]) { _, _ in }
    }

    func appRemoved(name: String, bundleId: String, leftovers: Int, bytes: UInt64) {
        let content = UNMutableNotificationContent()
        content.title = "\(name) a été supprimée"
        content.body = "\(leftovers) restes trouvés (\(Format.bytes(bytes))). Cliquer pour les nettoyer."
        content.userInfo = [Key.target: "uninstall", Key.bundleId: bundleId]
        post(content, id: "removed.\(bundleId)")
    }

    func lowSpace(available: Int64) {
        let content = UNMutableNotificationContent()
        content.title = "Espace disque faible"
        content.body = "Plus que \(Format.bytes(UInt64(max(available, 0)))) libres. Lance une analyse pour en récupérer."
        content.userInfo = [Key.target: "cleanup"]
        post(content, id: "low-space")
    }

    func memoryCritical(topName: String?, topBytes: UInt64?) {
        let content = UNMutableNotificationContent()
        content.title = "Mémoire saturée"
        if let topName, let topBytes {
            content.body = "\(topName) utilise \(Format.bytes(topBytes)). Cliquer pour voir ce qui occupe la mémoire."
        } else {
            content.body = "Cliquer pour voir ce qui occupe la mémoire."
        }
        content.userInfo = [Key.target: "memory"]
        post(content, id: "memory-critical")
    }

    func cleaned(bytes: UInt64, failures: Int, automatic: Bool = false) {
        let content = UNMutableNotificationContent()
        content.title = "\(Format.bytes(bytes)) libérés"
        content.body = failures == 0
            ? automatic
                ? "Nettoyage automatique : seuls les caches et fichiers qui se recréent ont été retirés."
                : "Nettoyage terminé : seuls les éléments sans risque ont été retirés."
            : "\(Format.counted(failures, "élément")) \(Format.agree(failures, "n'a pas pu être retiré", "n'ont pas pu être retirés")). Cliquer pour voir le détail."
        content.userInfo = [Key.target: "cleanup"]
        post(content, id: "cleaned")
    }

    func folderGrew(path: String, grown: Int64) {
        let content = UNMutableNotificationContent()
        content.title = "\((path as NSString).lastPathComponent) a pris \(Format.bytes(UInt64(max(grown, 0))))"
        content.body = "En un jour, dans \(path.abbreviatingHome). Cliquer pour voir ce qui prend la place."
        content.userInfo = [Key.target: "explorer"]
        post(content, id: "grew.\(path)")
    }

    private func post(_ content: UNMutableNotificationContent, id: String) {
        content.sound = .default
        center.add(UNNotificationRequest(identifier: id, content: content, trigger: nil))
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification
    ) async -> UNNotificationPresentationOptions {
        [.banner, .sound]
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        let info = response.notification.request.content.userInfo
        let target: Target? = switch info[Key.target] as? String {
        case "cleanup": .cleanup
        case "memory": .memory
        case "explorer": .explorer
        case "uninstall": (info[Key.bundleId] as? String).map { .uninstall(bundleId: $0) }
        default: nil
        }
        guard let target else { return }
        await MainActor.run { onOpen?(target) }
    }
}
