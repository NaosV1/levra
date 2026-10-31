import DebarrasBindings
import SwiftUI

struct AppsView: View {
    @Bindable var model: AppsModel
    @State private var dropTargeted = false

    var body: some View {
        PageScroll {
            PageHeader(title: "Applications", subtitle: subtitle) {
                SearchField(prompt: "Rechercher une app", text: $model.search)
                SegmentedControl(
                    options: AppsModel.Sort.allCases.map { ($0, $0.rawValue) },
                    selection: $model.sort
                )
                IconButton(icon: .refresh, help: "Actualiser") { Task { await model.load() } }
                    .keyboardShortcut("r", modifiers: .command)
                    .disabled(model.loading)
            }
        } content: {
            VStack(alignment: .leading, spacing: 16) {
                dropZone
                if model.loading && model.apps.isEmpty {
                    HStack { Spacer(); Spinner(size: 20); Spacer() }.padding(.vertical, 48)
                } else {
                    let apps = model.visibleApps
                    VStack(spacing: 0) {
                        ForEach(Array(apps.enumerated()), id: \.element.path) { index, app in
                            AppRow(app: app, isLast: index == apps.count - 1) { Task { await model.prepare(app) } }
                                .staggeredAppear(index: index)
                        }
                    }
                    .card()
                    .animation(Motion.snappy, value: apps.map(\.path))
                }
            }
        }
        .dropDestination(for: URL.self) { urls, _ in
            guard let url = urls.first(where: { $0.pathExtension == "app" }) else { return false }
            Task { await model.prepareDropped(url) }
            return true
        } isTargeted: { targeted in
            withAnimation(Motion.snappy) { dropTargeted = targeted }
        }
        .uninstallReview(model)
    }

    private var subtitle: String {
        let total = model.apps.reduce(0) { $0 + $1.bytes }
        return "\(model.apps.count) apps · \(Format.bytes(total))"
    }

    private var dropZone: some View {
        HStack(spacing: 12) {
            Icon(.download, size: 18)
                .foregroundStyle(dropTargeted ? Theme.accent : Theme.textTertiary)
            VStack(alignment: .leading, spacing: 2) {
                Text("Glisse une app ici pour la désinstaller complètement").textStyle(.bodyStrong)
                Text("L'app et tout ce qu'elle a laissé : préférences, caches, conteneurs, services.").textStyle(.secondary)
            }
            Spacer()
        }
        .padding(16)
        .background(dropTargeted ? Theme.accent.opacity(0.06) : .clear, in: RoundedRectangle(cornerRadius: Theme.radius))
        .overlay {
            RoundedRectangle(cornerRadius: Theme.radius)
                .strokeBorder(dropTargeted ? Theme.accent : Theme.borderStrong, style: StrokeStyle(lineWidth: 1, dash: [4, 4]))
        }
        .scaleEffect(dropTargeted ? 1.01 : 1)
    }
}

private struct AppRow: View {
    let app: AppRecord
    let isLast: Bool
    let onUninstall: () -> Void

    var body: some View {
        CardRow(isLast: isLast) {
            HStack(spacing: 12) {
                AppIcon(path: app.path, size: 28)
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 6) {
                        Text(app.name).textStyle(.bodyStrong).lineLimit(1)
                        if let version = app.version {
                            Text(version).textStyle(.caption)
                        }
                    }
                    Text(app.lastUsedLabel).textStyle(.secondary)
                }
                Spacer(minLength: 12)
                HoverReveal {
                    HStack(spacing: 4) {
                        IconButton(icon: .arrowUpRight, help: "Afficher dans le Finder") { SystemTrash.reveal(app.path) }
                        Button("Désinstaller…", action: onUninstall).buttonStyle(.app(.secondary, size: .small))
                    }
                }
                Text(Format.bytes(app.bytes))
                    .textStyle(.bodyStrong)
                    .monospacedDigit()
                    .frame(width: 76, alignment: .trailing)
            }
        }
        .onTapGesture(perform: onUninstall)
        .contextMenu {
            Button("Désinstaller…", action: onUninstall)
            Button("Afficher dans le Finder") { SystemTrash.reveal(app.path) }
        }
        .accessibilityElement(children: .contain)
        .accessibilityAction(named: "Désinstaller…", onUninstall)
        .accessibilityAction(named: "Afficher dans le Finder") { SystemTrash.reveal(app.path) }
    }
}
