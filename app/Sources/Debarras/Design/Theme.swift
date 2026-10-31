import AppKit
import SwiftUI

/// Colours and metrics: neutral greys, hairline borders, a single accent.
/// Every colour adapts to light and dark appearance.
enum Theme {
    static let background = Color(light: 0xFFFFFF, dark: 0x0E0E10)
    static let sidebar = Color(light: 0xF7F7F8, dark: 0x09090A)
    static let surface = Color(light: 0xFFFFFF, dark: 0x141416)
    static let surfaceHover = Color(light: 0xF4F4F5, dark: 0x1B1B1E)
    static let surfaceSelected = Color(light: 0xEBEBEE, dark: 0x232327)
    static let border = Color(light: 0x000000, lightAlpha: 0.08, dark: 0xFFFFFF, darkAlpha: 0.07)
    static let borderStrong = Color(light: 0x000000, lightAlpha: 0.16, dark: 0xFFFFFF, darkAlpha: 0.16)

    static let textPrimary = Color(light: 0x18181B, dark: 0xEDEDEF)
    static let textSecondary = Color(light: 0x5B5B66, dark: 0xA0A0AB)
    static let textTertiary = Color(light: 0x8E8E98, dark: 0x6A6A74)
    /// Text on the inverted primary button.
    static let textInverse = Color(light: 0xFFFFFF, dark: 0x0E0E10)

    static let accent = Color(light: 0x5E6AD2, dark: 0x7C86E2)
    static let success = Color(light: 0x16A34A, dark: 0x3FB950)
    static let warning = Color(light: 0xC2410C, dark: 0xF5A524)
    static let danger = Color(light: 0xDC2626, dark: 0xF26D6D)

    static let radius: CGFloat = 10
    static let radiusSmall: CGFloat = 6
    static let pagePadding: CGFloat = 28
    static let contentWidth: CGFloat = 1080
}

/// Named text styles, so screens never pick sizes ad hoc.
enum TextStyle {
    case pageTitle, heading, body, bodyStrong, secondary, caption, label, mono, metric

    var font: Font {
        switch self {
        case .pageTitle: .system(size: 22, weight: .semibold)
        case .heading: .system(size: 15, weight: .semibold)
        case .body: .system(size: 13)
        case .bodyStrong: .system(size: 13, weight: .medium)
        case .secondary: .system(size: 12)
        case .caption: .system(size: 11)
        case .label: .system(size: 11, weight: .medium)
        case .mono: .system(size: 11, design: .monospaced)
        case .metric: .system(size: 26, weight: .semibold).monospacedDigit()
        }
    }

    var color: Color {
        switch self {
        case .pageTitle, .heading, .body, .bodyStrong, .metric: Theme.textPrimary
        case .secondary, .label: Theme.textSecondary
        case .caption, .mono: Theme.textTertiary
        }
    }

    /// Large type is tightened, small labels slightly opened.
    var tracking: CGFloat {
        switch self {
        case .pageTitle, .metric: -0.5
        case .heading: -0.2
        case .label: 0.2
        case .body, .bodyStrong, .secondary, .caption, .mono: 0
        }
    }
}

extension View {
    func textStyle(_ style: TextStyle) -> some View {
        font(style.font).foregroundStyle(style.color).tracking(style.tracking)
    }

    /// A hairline-bordered panel on the surface colour.
    func card(padding: CGFloat = 0) -> some View {
        self.padding(padding)
            .background(Theme.surface, in: RoundedRectangle(cornerRadius: Theme.radius))
            .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(Theme.border))
    }
}

extension Color {
    init(light: UInt32, lightAlpha: Double = 1, dark: UInt32, darkAlpha: Double = 1) {
        self.init(nsColor: NSColor(name: nil) { appearance in
            let isDark = appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
            return NSColor(hex: isDark ? dark : light, alpha: isDark ? darkAlpha : lightAlpha)
        })
    }
}

private extension NSColor {
    convenience init(hex: UInt32, alpha: Double) {
        self.init(
            srgbRed: CGFloat((hex >> 16) & 0xFF) / 255,
            green: CGFloat((hex >> 8) & 0xFF) / 255,
            blue: CGFloat(hex & 0xFF) / 255,
            alpha: alpha
        )
    }
}
