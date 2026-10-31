import ServiceManagement
import SwiftUI

struct SettingsView: View {
    @Environment(AppModel.self) private var model
    @State private var launchAtLogin = SMAppService.mainApp.status == .enabled
    @State private var loginError: String?

    var body: some View {
        @Bindable var model = model
        @Bindable var updates = model.updates
        Form {
            Section("Surveillance") {
                Toggle("Repérer les apps supprimées et proposer d'effacer leurs restes", isOn: $model.watchUninstalls)
                Picker("Alerter quand il reste moins de", selection: $model.lowSpaceGB) {
                    ForEach([10, 20, 50, 100], id: \.self) { Text("\($0) Go libres").tag($0) }
                }
            }
            Section {
                Picker("Nettoyage automatique", selection: $model.autoClean) {
                    ForEach(AppModel.AutoClean.allCases) { Text($0.title).tag($0) }
                }
                if let next = model.nextAutoClean {
                    LabeledContent("Prochain nettoyage") {
                        Text(next <= .now ? "dans l'heure" : next.formatted(.relative(presentation: .named)))
                    }
                }
            } header: {
                Text("Nettoyage")
            } footer: {
                Text("Seuls les éléments sans risque partent (caches, fichiers qui se recréent), comme le nettoyage rapide de la barre des menus. Une notification dit ce qui a été libéré.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Section("Mises à jour") {
                Toggle("Rechercher les mises à jour chaque jour", isOn: $updates.checksAutomatically)
                LabeledContent("Version \(updates.version)") {
                    Button("Rechercher maintenant") { updates.check() }
                        .disabled(!updates.canCheck)
                }
            }
            Section("Démarrage") {
                Toggle("Ouvrir Levra à la connexion", isOn: $launchAtLogin)
                    .onChange(of: launchAtLogin) { _, enabled in setLaunchAtLogin(enabled) }
                if let loginError {
                    Text(loginError).font(.caption).foregroundStyle(.red)
                }
            }
            Section("Autorisations macOS") {
                PermissionRow(
                    title: "Accès complet au disque",
                    detail: "Pour analyser la corbeille, les sauvegardes iPhone, Mail et Messages.",
                    pane: .fullDiskAccess
                )
                PermissionRow(
                    title: "Automatisation › Finder",
                    detail: "Pour faire mettre à la corbeille par le Finder ce qui est verrouillé ; il demande alors ton mot de passe.",
                    pane: .automation
                )
                PermissionRow(
                    title: "Gestion des apps",
                    detail: "Pour mettre à la corbeille les apps qui t'appartiennent sans passer par le Finder.",
                    pane: .appManagement
                )
            }
            Section("À propos") {
                LabeledContent("Icônes", value: "PapIcons, licence MIT © 2025 PapTools")
                LabeledContent("Mises à jour", value: "Sparkle, licence MIT © Andy Matuschak et contributeurs")
            }
        }
        .formStyle(.grouped)
        .frame(width: 520)
        .fixedSize(horizontal: false, vertical: true)
    }

    private func setLaunchAtLogin(_ enabled: Bool) {
        do {
            if enabled {
                try SMAppService.mainApp.register()
            } else {
                try SMAppService.mainApp.unregister()
            }
            loginError = nil
        } catch {
            loginError = "macOS a refusé : \(error.localizedDescription)"
            launchAtLogin = SMAppService.mainApp.status == .enabled
        }
    }
}

private struct PermissionRow: View {
    let title: String
    let detail: String
    let pane: PrivacyPane

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                Text(detail).font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
            Button("Ouvrir") { pane.open() }
        }
    }
}
