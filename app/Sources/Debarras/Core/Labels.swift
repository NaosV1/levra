import DebarrasBindings
import SwiftUI

enum Format {
    /// Decimal units, like the Finder ("43,7 Go"), always in figures
    /// (the formatter would otherwise write "Zéro ko").
    static func bytes(_ value: UInt64) -> String {
        let formatter = ByteCountFormatter()
        formatter.countStyle = .file
        formatter.allowsNonnumericFormatting = false
        return formatter.string(fromByteCount: Int64(clamping: value))
    }

    static func date(_ unixSeconds: UInt64?) -> Date? {
        unixSeconds.map { Date(timeIntervalSince1970: TimeInterval($0)) }
    }

    /// "il y a 3 mois".
    static func relative(_ unixSeconds: UInt64?) -> String? {
        guard let date = date(unixSeconds) else { return nil }
        return date.formatted(.relative(presentation: .named))
    }

    static func count(_ value: UInt64) -> String {
        value.formatted(.number)
    }
}

extension AppRecord {
    var lastUsedLabel: String {
        Format.relative(lastUsed).map { "Ouverte \($0)" } ?? "Jamais ouverte (selon Spotlight)"
    }
}

extension DebarrasBindings.Category {
    var label: String {
        switch self {
        case .projectBuilds: "Builds de projets"
        case .gitWorktrees: "Worktrees git"
        case .packageCaches: "Caches de paquets"
        case .appCaches: "Caches d'applications"
        case .xcode: "Xcode"
        case .android: "Android"
        case .docker: "Docker"
        case .personalFiles: "Fichiers personnels"
        }
    }

    var icon: Papicon {
        switch self {
        case .projectBuilds: .sourceCode
        case .gitWorktrees: .gitBranch
        case .packageCaches: .package
        case .appCaches: .layers
        case .xcode: .smartphone
        case .android: .android
        case .docker: .cube
        case .personalFiles: .user
        }
    }
}

extension ScanStep {
    var label: String {
        switch self {
        case .projects: "Projets de développement"
        case .caches: "Caches des apps"
        case .xcode: "Xcode et simulateurs"
        case .android: "Android"
        case .docker: "Docker"
        case .personalFiles: "Corbeille et installeurs"
        }
    }
}

extension Safety {
    var label: String {
        switch self {
        case .safe: "Sûr"
        case .review: "À vérifier"
        case .careful: "Prudence"
        }
    }

    var tone: Badge.Tone {
        switch self {
        case .safe: .success
        case .review: .warning
        case .careful: .danger
        }
    }
}

extension MemoryPressure {
    var label: String {
        switch self {
        case .normal: "Normale"
        case .warning: "Élevée"
        case .critical: "Critique"
        }
    }

    var tone: Badge.Tone {
        switch self {
        case .normal: .success
        case .warning: .warning
        case .critical: .danger
        }
    }

    var explanation: String {
        switch self {
        case .normal: "macOS a toute la mémoire dont il a besoin."
        case .warning: "macOS compresse et utilise le swap : les apps peuvent ralentir."
        case .critical: "La mémoire manque : quitte les apps les plus gourmandes."
        }
    }
}

extension DebarrasBindings.ProcessKind {
    /// Shown under groups Débarras cannot quit, or that need explaining.
    var hint: String? {
        switch self {
        case .app: nil
        case .tool: "Programme en ligne de commande"
        case .virtualMachine: "Lancée par une app de virtualisation (Docker Desktop, l'app Claude…) : quitte cette app pour libérer la mémoire."
        case .system: "Composant de macOS"
        }
    }

    var icon: Papicon {
        switch self {
        case .app: .grid
        case .tool: .commandLine
        case .virtualMachine: .cube
        case .system: .system
        }
    }
}

extension ActionKind {
    var label: String {
        switch self {
        case .trash: "Mis à la corbeille"
        case .delete: "Supprimé définitivement"
        case .command: "Nettoyé par son outil"
        case .manual: "À faire soi-même"
        }
    }
}

extension LeftoverKind {
    var label: String {
        switch self {
        case .application: "Application"
        case .support: "Données de l'app"
        case .cache: "Caches"
        case .preferences: "Préférences"
        case .container: "Conteneurs"
        case .groupContainer: "Conteneurs partagés"
        case .savedState: "État des fenêtres"
        case .webData: "Données web"
        case .logs: "Journaux"
        case .scripts: "Scripts"
        case .launchAgent: "Agents de démarrage"
        case .launchDaemon: "Services système"
        case .helperTool: "Outils privilégiés"
        case .temporary: "Fichiers temporaires"
        case .receipt: "Reçus d'installation"
        }
    }

    static let ordered: [LeftoverKind] = [
        .application, .support, .cache, .preferences, .container, .groupContainer, .savedState, .webData,
        .logs, .scripts, .launchAgent, .launchDaemon, .helperTool, .temporary, .receipt,
    ]
}

extension JobScope {
    var label: String {
        switch self {
        case .userAgent: "Ta session"
        case .agent: "Toutes les sessions"
        case .daemon: "Service système"
        }
    }
}

extension JobSchedule {
    /// When launchd starts the job; agents load at login, daemons at startup.
    func label(scope: JobScope) -> String {
        switch self {
        case .atLoad: scope == .daemon ? "Au démarrage du Mac" : "À l'ouverture de session"
        case .keepAlive: "En permanence"
        case let .interval(seconds):
            switch seconds {
            case 3600: "Toutes les heures"
            case 86400: "Tous les jours"
            case let s where s >= 3600 && s % 3600 == 0: "Toutes les \(s / 3600) h"
            case let s where s >= 60: "Toutes les \(s / 60) min"
            default: "Toutes les \(seconds) s"
            }
        case .calendar: "À heures fixes"
        case .onDemand: "À la demande"
        case .unknown: "Réglages lisibles seulement par macOS"
        }
    }
}

extension MatchReason {
    /// Why the file is attributed to the app.
    var label: String {
        switch self {
        case .bundleId: "Identifiant de l'app"
        case .name: "Nom de l'app"
        case .team: "Même éditeur (signature)"
        case .vendor: "Même éditeur (identifiant)"
        case .similar: "Nom proche"
        }
    }

    /// Why a match this loose is left unchecked.
    var caution: String? {
        switch self {
        case .bundleId, .name, .team: nil
        case .vendor: "Décoché par prudence : il porte l'identifiant de l'éditeur, pas celui de l'app. Il vient sans doute d'une autre de ses apps, désinstallée ou rangée ailleurs."
        case .similar: "Décoché par prudence : seul son nom ressemble à celui de l'app."
        }
    }
}
