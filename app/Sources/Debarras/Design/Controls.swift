import SwiftUI

struct AppButtonStyle: ButtonStyle {
    enum Kind {
        /// Inverted (white on dark, black on light): the one main action.
        case primary
        case secondary
        case ghost
        case danger
    }

    enum Size {
        case regular, small
    }

    var kind: Kind
    var size: Size = .regular

    func makeBody(configuration: Configuration) -> some View {
        AppButton(configuration: configuration, kind: kind, size: size)
    }
}

extension ButtonStyle where Self == AppButtonStyle {
    static func app(_ kind: AppButtonStyle.Kind, size: AppButtonStyle.Size = .regular) -> AppButtonStyle {
        AppButtonStyle(kind: kind, size: size)
    }
}

private struct AppButton: View {
    let configuration: ButtonStyleConfiguration
    let kind: AppButtonStyle.Kind
    let size: AppButtonStyle.Size
    @Environment(\.isEnabled) private var isEnabled
    @State private var hovering = false

    var body: some View {
        configuration.label
            .font(.system(size: size == .small ? 12 : 13, weight: .medium))
            .lineLimit(1)
            .padding(.horizontal, size == .small ? 8 : 12)
            .frame(height: size == .small ? 24 : 30)
            .foregroundStyle(foreground)
            .background(background, in: RoundedRectangle(cornerRadius: Theme.radiusSmall))
            .overlay {
                if kind == .secondary {
                    RoundedRectangle(cornerRadius: Theme.radiusSmall).strokeBorder(Theme.border)
                }
            }
            .contentShape(Rectangle())
            .scaleEffect(configuration.isPressed ? 0.97 : 1)
            .opacity(isEnabled ? 1 : 0.4)
            .animation(Motion.quick, value: configuration.isPressed)
            .animation(Motion.hover, value: hovering)
            .onHover { hovering = $0 && isEnabled }
    }

    private var foreground: Color {
        switch kind {
        case .primary: Theme.textInverse
        case .secondary: Theme.textPrimary
        case .ghost: hovering ? Theme.textPrimary : Theme.textSecondary
        case .danger: .white
        }
    }

    private var background: Color {
        switch kind {
        case .primary: Theme.textPrimary.opacity(hovering ? 0.88 : 1)
        case .secondary: hovering ? Theme.surfaceHover : Theme.surface
        case .ghost: hovering ? Theme.surfaceHover : .clear
        case .danger: Theme.danger.opacity(hovering ? 0.88 : 1)
        }
    }
}

/// Icon with a title, the label of most buttons.
struct ButtonLabel: View {
    let title: String
    let icon: Papicon

    init(_ title: String, icon: Papicon) {
        self.title = title
        self.icon = icon
    }

    var body: some View {
        HStack(spacing: 6) {
            Icon(icon, size: 14)
            Text(title)
        }
    }
}

/// Square icon-only button for row actions; always carries a help text.
struct IconButton: View {
    let icon: Papicon
    let help: String
    var tint: Color = Theme.textSecondary
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Icon(icon, size: 15)
                .foregroundStyle(tint)
                .frame(width: 26, height: 26)
        }
        .buttonStyle(.app(.ghost, size: .small))
        .help(help)
        .accessibilityLabel(help)
    }
}

struct Checkbox: View {
    @Binding var isOn: Bool
    @Environment(\.isEnabled) private var isEnabled

    var body: some View {
        Button {
            withAnimation(Motion.quick) { isOn.toggle() }
        } label: {
            RoundedRectangle(cornerRadius: 4)
                .fill(isOn ? Theme.accent : Theme.surface)
                .overlay {
                    RoundedRectangle(cornerRadius: 4).strokeBorder(isOn ? Theme.accent : Theme.borderStrong)
                }
                .overlay {
                    if isOn {
                        Icon(.check, size: 12, weight: 1.6)
                            .foregroundStyle(.white)
                            .transition(.scale(scale: 0.6).combined(with: .opacity))
                    }
                }
                .frame(width: 16, height: 16)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .opacity(isEnabled ? 1 : 0.35)
        .accessibilityAddTraits(.isToggle)
        .accessibilityValue(isOn ? "coché" : "non coché")
    }
}

struct Badge: View {
    enum Tone {
        case neutral, accent, success, warning, danger

        var color: Color {
            switch self {
            case .neutral: Theme.textSecondary
            case .accent: Theme.accent
            case .success: Theme.success
            case .warning: Theme.warning
            case .danger: Theme.danger
            }
        }
    }

    let text: String
    var tone: Tone = .neutral

    var body: some View {
        Text(text)
            .font(.system(size: 11, weight: .medium))
            .foregroundStyle(tone.color)
            .padding(.horizontal, 6)
            .frame(height: 18)
            .background(tone.color.opacity(0.12), in: RoundedRectangle(cornerRadius: 4))
    }
}

/// A row of options; the selected one sits on a pill that glides between them.
struct SegmentedControl<Value: Hashable>: View {
    let options: [(value: Value, title: String)]
    @Binding var selection: Value
    @Namespace private var pill

    var body: some View {
        HStack(spacing: 2) {
            ForEach(options, id: \.value) { option in
                let selected = option.value == selection
                Button {
                    withAnimation(Motion.glide) { selection = option.value }
                } label: {
                    Text(option.title)
                        .font(.system(size: 12, weight: .medium))
                        .foregroundStyle(selected ? Theme.textPrimary : Theme.textSecondary)
                        .padding(.horizontal, 10)
                        .frame(height: 24)
                        .background {
                            if selected {
                                RoundedRectangle(cornerRadius: 5)
                                    .fill(Theme.surface)
                                    .shadow(color: .black.opacity(0.08), radius: 1, y: 1)
                                    .matchedGeometryEffect(id: "pill", in: pill)
                            }
                        }
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
        }
        .padding(2)
        .background(Theme.surfaceHover, in: RoundedRectangle(cornerRadius: 7))
    }
}

struct SearchField: View {
    let prompt: String
    @Binding var text: String
    @FocusState private var focused: Bool

    var body: some View {
        HStack(spacing: 6) {
            Icon(.search, size: 14).foregroundStyle(Theme.textTertiary)
            TextField(prompt, text: $text)
                .textFieldStyle(.plain)
                .font(.system(size: 13))
                .focused($focused)
            if !text.isEmpty {
                Button {
                    text = ""
                } label: {
                    Icon(.cancel, size: 12).foregroundStyle(Theme.textTertiary)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Effacer la recherche")
            }
        }
        .padding(.horizontal, 10)
        .frame(width: 220, height: 30)
        .background(Theme.surface, in: RoundedRectangle(cornerRadius: Theme.radiusSmall))
        .overlay {
            RoundedRectangle(cornerRadius: Theme.radiusSmall)
                .strokeBorder(focused ? Theme.accent.opacity(0.6) : Theme.border)
        }
        .animation(Motion.hover, value: focused)
    }
}
