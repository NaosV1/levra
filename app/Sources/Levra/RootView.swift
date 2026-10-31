import SwiftUI

struct RootView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        @Bindable var model = model
        HStack(spacing: 0) {
            Sidebar()
            Rectangle().fill(Theme.border).frame(width: 1)
            ZStack {
                page
                    .id(model.section)
                    .transition(.lift(reduceMotion: reduceMotion))
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Theme.background)
        }
        .ignoresSafeArea(.container, edges: .top)
        .background { KeyboardNavigation() }
        .sheet(isPresented: $model.showsWelcome) { OnboardingView() }
    }

    @ViewBuilder
    private var page: some View {
        switch model.section {
        case .home: HomeView()
        case .cleanup: CleanupView(model: model.cleanup)
        case .explorer: ExplorerView(model: model.explorer)
        case .largeFiles: LargeFilesView(explorer: model.explorer)
        case .duplicates: DuplicatesView(model: model.duplicates)
        case .memory: MemoryView(model: model.memory)
        case .apps: AppsView(model: model.apps)
        case .debloat: DebloatView(model: model.debloat, apps: model.apps)
        }
    }
}

extension AppModel.Section {
    /// ⌘1…⌘8, in sidebar order.
    var shortcut: KeyEquivalent {
        KeyEquivalent(Character(String((Self.allCases.firstIndex(of: self) ?? 0) + 1)))
    }
}

/// ⌘1…⌘8 switch pages instantly: keyboard actions are never animated.
private struct KeyboardNavigation: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        ForEach(AppModel.Section.allCases) { section in
            Button(section.title) { model.section = section }
                .keyboardShortcut(section.shortcut, modifiers: .command)
        }
        .opacity(0)
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }
}

private struct Sidebar: View {
    @Environment(AppModel.self) private var model
    @Namespace private var selection

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 8) {
                LevraMark(size: 22)
                Text("Levra").font(.system(size: 13, weight: .semibold)).foregroundStyle(Theme.textPrimary)
            }
            .padding(.horizontal, 10)
            .padding(.top, 44)
            .padding(.bottom, 18)

            SidebarItem(section: .home, selected: model.section == .home, namespace: selection) {
                withAnimation(Motion.glide) { model.section = .home }
            }
            .padding(.bottom, 18)
            group("Espace disque", [.cleanup, .explorer, .largeFiles, .duplicates])
            group("Système", [.memory, .apps, .debloat])

            Spacer()

            VStack(alignment: .leading, spacing: 12) {
                if let disk = model.disk {
                    Meter(
                        icon: .hardDrive,
                        label: "Disque",
                        value: "\(Format.bytes(UInt64(max(disk.available, 0)))) libres",
                        fraction: disk.used,
                        tint: disk.used > 0.9 ? Theme.danger : Theme.textSecondary
                    )
                }
                if let memory = model.memoryStats {
                    Meter(
                        icon: .chip,
                        label: "Mémoire",
                        value: "Pression \(memory.pressure.label.lowercased())",
                        fraction: memory.total > 0 ? Double(memory.used) / Double(memory.total) : 0,
                        tint: memory.pressure.tone.color
                    )
                }
                SettingsLink {
                    HStack(spacing: 8) {
                        Icon(.settings, size: 15)
                        Text("Réglages")
                    }
                    .font(.system(size: 13))
                    .foregroundStyle(Theme.textSecondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .keyboardShortcut(",", modifiers: .command)
            }
            .padding(.horizontal, 10)
            .padding(.bottom, 16)
        }
        .padding(.horizontal, 8)
        .frame(width: 228)
        .background(Theme.sidebar)
    }

    private func group(_ title: String, _ sections: [AppModel.Section]) -> some View {
        VStack(alignment: .leading, spacing: 1) {
            Text(title)
                .textStyle(.label)
                .foregroundStyle(Theme.textTertiary)
                .padding(.horizontal, 10)
                .padding(.bottom, 6)
            ForEach(sections) { section in
                SidebarItem(section: section, selected: model.section == section, namespace: selection) {
                    withAnimation(Motion.glide) { model.section = section }
                }
            }
        }
        .padding(.bottom, 18)
    }
}

private struct SidebarItem: View {
    let section: AppModel.Section
    let selected: Bool
    let namespace: Namespace.ID
    let action: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 9) {
                Icon(section.icon, size: 16)
                Text(section.title).font(.system(size: 13, weight: selected ? .medium : .regular))
                Spacer()
                Text("⌘\(String(section.shortcut.character))")
                    .textStyle(.caption)
                    .opacity(hovering ? 1 : 0)
            }
            .foregroundStyle(selected ? Theme.textPrimary : Theme.textSecondary)
            .padding(.horizontal, 10)
            .frame(height: 30)
            .background {
                if selected {
                    RoundedRectangle(cornerRadius: Theme.radiusSmall)
                        .fill(Theme.surfaceSelected)
                        .matchedGeometryEffect(id: "selection", in: namespace)
                } else if hovering {
                    RoundedRectangle(cornerRadius: Theme.radiusSmall).fill(Theme.surfaceHover)
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(section.summary)
        .onHover { hovering = $0 }
        .animation(Motion.hover, value: hovering)
        .accessibilityAddTraits(selected ? .isSelected : [])
        .accessibilityHint(section.summary)
    }
}

private struct Meter: View {
    let icon: Papicon
    let label: String
    let value: String
    let fraction: Double
    let tint: Color

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 6) {
                Icon(icon, size: 13).foregroundStyle(Theme.textTertiary)
                Text(label).textStyle(.caption)
                Spacer()
                Text(value).textStyle(.caption).contentTransition(.numericText())
            }
            SizeBar(fraction: fraction, width: 188, tint: tint)
        }
    }
}
