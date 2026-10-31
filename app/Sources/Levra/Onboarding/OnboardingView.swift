import SwiftUI

/// Shown once, at first launch: what Levra does, and the one permission
/// that lets it see everything.
struct OnboardingView: View {
    @Environment(\.dismiss) private var dismiss
    @State private var granted = FullDiskAccess.isGranted

    var body: some View {
        VStack(alignment: .leading, spacing: 24) {
            VStack(alignment: .leading, spacing: 8) {
                LevraMark(size: 44)
                Text("Bienvenue dans Levra")
                    .font(.system(size: 24, weight: .semibold))
                    .tracking(-0.5)
                    .foregroundStyle(Theme.textPrimary)
                    .padding(.top, 6)
                Text("Levra fait de la place sur ton Mac sans rien casser : rien ne part sans ta confirmation, et presque tout passe par la corbeille.")
                    .textStyle(.body)
                    .foregroundStyle(Theme.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }

            VStack(alignment: .leading, spacing: 14) {
                Feature(icon: .clean, title: "Nettoyage", text: "Ce qui se recrée tout seul : caches, fichiers de développement.")
                Feature(icon: .grid, title: "Applications", text: "Désinstalle une app avec tout ce qu'elle a laissé.")
                Feature(icon: .chip, title: "Mémoire et arrière-plan", text: "Ce qui occupe ta mémoire et tourne sans que tu le saches.")
            }

            permission

            HStack {
                Spacer()
                if !granted {
                    Button("Plus tard") { dismiss() }
                        .buttonStyle(.app(.ghost))
                }
                Button("Commencer") { dismiss() }
                    .buttonStyle(.app(.primary, size: .large))
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding(32)
        .frame(width: 560)
        .background(Theme.background)
        // The user grants access in System Settings: follow it from here.
        .task {
            while !granted, !Task.isCancelled {
                try? await Task.sleep(for: .seconds(1.5))
                withAnimation(Motion.snappy) { granted = FullDiskAccess.isGranted }
            }
        }
    }

    private var permission: some View {
        HStack(alignment: .top, spacing: 14) {
            Icon(granted ? .checkCircle : .lock, size: 20, weight: 1.2)
                .foregroundStyle(granted ? Theme.success : Theme.textSecondary)
                .contentTransition(.opacity)
            VStack(alignment: .leading, spacing: 4) {
                Text(granted ? "Accès complet au disque accordé" : "Accès complet au disque")
                    .textStyle(.bodyStrong)
                Text(granted
                    ? "Levra voit aussi la corbeille, Mail, Messages et les sauvegardes d'iPhone."
                    : "Sans lui, Levra ne voit pas la corbeille, Mail, Messages, les sauvegardes d'iPhone ni les données des apps. Ajoute Levra dans la liste qui s'ouvre, puis reviens ici.")
                    .textStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 12)
            if !granted {
                Button("Autoriser") { PrivacyPane.fullDiskAccess.open() }
                    .buttonStyle(.app(.secondary))
            }
        }
        .padding(16)
        .card()
        .animation(Motion.snappy, value: granted)
    }
}

private struct Feature: View {
    let icon: Papicon
    let title: String
    let text: String

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Icon(icon, size: 18).foregroundStyle(Theme.accent).frame(width: 22)
            VStack(alignment: .leading, spacing: 2) {
                Text(title).textStyle(.bodyStrong)
                Text(text).textStyle(.secondary)
            }
        }
        .accessibilityElement(children: .combine)
    }
}
