import LevraBindings
import SwiftUI

extension View {
    /// Shows the review of `model.plan` once it is prepared, or why it could
    /// not be. `onDismiss` runs when the review closes.
    func uninstallReview(_ model: AppsModel, onDismiss: (() -> Void)? = nil) -> some View {
        sheet(isPresented: Binding(get: { model.plan != nil }, set: { if !$0 { model.plan = nil } }), onDismiss: onDismiss) {
            UninstallView(model: model)
        }
        .alert(
            "Impossible de préparer la désinstallation",
            isPresented: Binding(get: { model.error != nil }, set: { if !$0 { model.dismissError() } })
        ) {
            Button("OK") { model.dismissError() }
        } message: {
            Text(model.error ?? "")
        }
    }
}

/// Everything an app leaves behind, grouped by kind, before it all goes
/// to the Trash.
struct UninstallView: View {
    @Bindable var model: AppsModel
    @Environment(AppModel.self) private var app
    @Environment(\.dismiss) private var dismiss
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        if let plan = model.plan {
            VStack(spacing: 0) {
                header(plan)
                Rectangle().fill(Theme.border).frame(height: 1)
                ScrollView {
                    VStack(alignment: .leading, spacing: 16) {
                        if let outcome = plan.outcome {
                            UninstallResultView(plan: plan, outcome: outcome)
                                .transition(.lift(reduceMotion: reduceMotion))
                        } else {
                            notices(plan)
                            items(plan)
                        }
                    }
                    .padding(20)
                }
                footer(plan)
            }
            .frame(width: 760, height: 600)
            .background(Theme.background)
            .animation(Motion.snappy, value: plan.outcome)
        }
    }

    private func header(_ plan: AppsModel.Plan) -> some View {
        HStack(spacing: 14) {
            AppIcon(path: plan.app.path, size: 44)
            VStack(alignment: .leading, spacing: 3) {
                Text("Désinstaller \(plan.app.name)").textStyle(.heading)
                Text(plan.app.bundleId).textStyle(.mono)
            }
            Spacer()
            // Once done, the result below gives the figures.
            if plan.outcome == nil {
                VStack(alignment: .trailing, spacing: 3) {
                    Text(Format.bytes(plan.selectedBytes))
                        .font(.system(size: 18, weight: .semibold).monospacedDigit())
                        .foregroundStyle(Theme.textPrimary)
                        .contentTransition(.numericText())
                    Text("\(plan.selection.count) sur \(plan.items.count) éléments").textStyle(.caption)
                }
                .animation(Motion.snappy, value: plan.selectedBytes)
            }
        }
        .padding(20)
    }

    @ViewBuilder
    private func notices(_ plan: AppsModel.Plan) -> some View {
        if !plan.appPresent {
            Banner(tone: .neutral, icon: .info, title: "L'app a déjà été supprimée", message: "Voici ce qu'elle a laissé.")
        }
        if plan.running {
            Banner(tone: .warning, icon: .alert, title: "\(plan.app.name) est ouverte", message: "Quitte-la avant de la désinstaller.") {
                Button("Quitter \(plan.app.name)") { Task { await model.quitApp() } }
                    .buttonStyle(.app(.secondary, size: .small))
            }
        }
        if let cask = plan.brewCask {
            HStack(spacing: 12) {
                Checkbox(isOn: Binding(get: { model.plan?.useBrew ?? false }, set: { model.plan?.useBrew = $0 }))
                VStack(alignment: .leading, spacing: 3) {
                    Text("Désinstaller d'abord avec Homebrew").textStyle(.bodyStrong)
                    Text("brew uninstall --cask --zap \(cask)").textStyle(.mono)
                }
                Spacer()
            }
            .padding(14)
            .card()
        }
        let closed = plan.items.filter(\.needsFullDiskAccess).count
        if closed > 0 {
            Banner(
                tone: .warning,
                icon: .lock,
                title: "\(Format.counted(closed, "conteneur")) \(Format.agree(closed, "protégé", "protégés")) par macOS, \(Format.agree(closed, "laissé", "laissés")) de côté",
                message: "macOS ferme les données des autres apps, au Finder aussi. Donne l'accès complet au disque à Levra, puis rouvre cette désinstallation pour les retirer."
            ) {
                Button("Ouvrir les réglages") { PrivacyPane.fullDiskAccess.open() }
                    .buttonStyle(.app(.secondary, size: .small))
            }
        }
        if plan.items.contains(where: \.needsAuthorization) {
            Banner(
                tone: .neutral,
                icon: .lock,
                title: "Le Finder demandera ton mot de passe pour les éléments verrouillés",
                message: "Ils appartiennent au système ou sont protégés par macOS : c'est le Finder qui les met à la corbeille, comme si tu les y glissais toi-même."
            )
        }
    }

    private func items(_ plan: AppsModel.Plan) -> some View {
        ForEach(LeftoverKind.ordered, id: \.self) { kind in
            let items = plan.items.filter { $0.kind == kind }
            if !items.isEmpty {
                VStack(alignment: .leading, spacing: 8) {
                    SectionHeader(title: kind.label, trailing: Format.bytes(items.reduce(0) { $0 + $1.bytes }))
                    VStack(spacing: 0) {
                        ForEach(Array(items.enumerated()), id: \.element.id) { index, item in
                            LeftoverRow(item: item, isSelected: binding(for: item), isLast: index == items.count - 1)
                        }
                    }
                    .card()
                }
            }
        }
    }

    private func footer(_ plan: AppsModel.Plan) -> some View {
        ActionBar {
            if plan.working {
                Spinner(size: 14)
                Text("Désinstallation…").textStyle(.secondary)
            }
            Spacer()
            if plan.outcome == nil {
                Button("Annuler") { dismiss() }
                    .buttonStyle(.app(.ghost))
                    .keyboardShortcut(.cancelAction)
                Button {
                    Task {
                        await model.uninstall()
                        app.refreshDisk()
                    }
                } label: {
                    ButtonLabel("Mettre à la corbeille", icon: .delete)
                }
                .buttonStyle(.app(.danger))
                .disabled(plan.selection.isEmpty || plan.running || plan.working)
            } else {
                if (plan.outcome?.completed ?? 0) > 0 {
                    Button { SystemTrash.open() } label: { ButtonLabel("Ouvrir la corbeille", icon: .delete) }
                        .buttonStyle(.app(.ghost))
                }
                Button("Terminé") { dismiss() }
                    .buttonStyle(.app(.primary))
                    .keyboardShortcut(.defaultAction)
            }
        }
    }

    private func binding(for item: LeftoverInfo) -> Binding<Bool> {
        Binding(
            get: { model.plan?.selection.contains(item.id) ?? false },
            set: { selected in
                if selected { model.plan?.selection.insert(item.id) } else { model.plan?.selection.remove(item.id) }
            }
        )
    }
}

private struct LeftoverRow: View {
    let item: LeftoverInfo
    @Binding var isSelected: Bool
    let isLast: Bool

    var body: some View {
        CardRow(isLast: isLast) {
            HStack(alignment: .top, spacing: 12) {
                Checkbox(isOn: $isSelected).padding(.top, 1)
                VStack(alignment: .leading, spacing: 4) {
                    HStack(spacing: 6) {
                        if item.needsAuthorization || item.needsFullDiskAccess {
                            Icon(.lock, size: 12).foregroundStyle(Theme.textTertiary)
                        }
                        Text(item.path.abbreviatingHome).textStyle(.body).lineLimit(1).truncationMode(.middle)
                    }
                    HStack(spacing: 8) {
                        Badge(text: item.reason.label, tone: reasonTone)
                        if let note = item.note {
                            Text(note).textStyle(.caption).foregroundStyle(Theme.warning)
                        }
                    }
                    if let caution = item.reason.caution {
                        Text(caution).textStyle(.caption).fixedSize(horizontal: false, vertical: true)
                    }
                }
                Spacer(minLength: 12)
                HoverReveal {
                    IconButton(icon: .arrowUpRight, help: "Afficher dans le Finder") { SystemTrash.reveal(item.path) }
                }
                Text(Format.bytes(item.bytes)).textStyle(.bodyStrong).monospacedDigit()
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityAction(named: "Afficher dans le Finder") { SystemTrash.reveal(item.path) }
    }

    private var reasonTone: Badge.Tone {
        switch item.reason {
        case .bundleId, .name: .neutral
        case .team: .accent
        case .vendor, .similar: .warning
        }
    }
}
