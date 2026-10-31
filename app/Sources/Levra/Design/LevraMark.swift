import SwiftUI

/// Levra's mark: a glossy disk drive with a sparkle, on a pink to blue
/// tile. Drawn here once, for the app (sidebar, welcome, menu bar) and for
/// the app icon, which scripts/artwork.swift renders from this view.
/// Nothing else of the app is used here, so that the script can compile
/// this file alone.
struct LevraMark: View {
    var size: CGFloat

    var body: some View {
        let corner = size * 0.225
        ZStack {
            RoundedRectangle(cornerRadius: corner, style: .continuous)
                .fill(LinearGradient(colors: [Self.pink, Self.violet, Self.blue], startPoint: .topLeading, endPoint: .bottomTrailing))
            // une lumière douce en haut à gauche, le reste de la tuile s'assombrit tt seul
            RoundedRectangle(cornerRadius: corner, style: .continuous)
                .fill(RadialGradient(
                    colors: [.white.opacity(0.2), .white.opacity(0)],
                    center: UnitPoint(x: 0.25, y: 0.1),
                    startRadius: 0,
                    endRadius: size * 0.7
                ))
            RoundedRectangle(cornerRadius: corner, style: .continuous)
                .strokeBorder(.white.opacity(0.18), lineWidth: max(size * 0.006, 0.5))

            Drive(size: size)
                .offset(y: size * 0.06)
            Sparkle()
                .fill(.white)
                .frame(width: size * 0.2, height: size * 0.2)
                .shadow(color: .white.opacity(0.8), radius: size * 0.03)
                .offset(x: size * 0.22, y: -size * 0.21)
            Sparkle()
                .fill(.white.opacity(0.85))
                .frame(width: size * 0.09, height: size * 0.09)
                .shadow(color: .white.opacity(0.7), radius: size * 0.015)
                .offset(x: size * 0.05, y: -size * 0.3)
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }

    private static let pink = Color(red: 0xFF / 255, green: 0x5C / 255, blue: 0xA8 / 255)
    private static let violet = Color(red: 0x8B / 255, green: 0x5C / 255, blue: 0xF6 / 255)
    private static let blue = Color(red: 0x3B / 255, green: 0x6E / 255, blue: 0xF6 / 255)
}

/// The drive: a slab of frosted glass, lit from above, with its light.
private struct Drive: View {
    let size: CGFloat

    var body: some View {
        let width = size * 0.6
        let height = size * 0.32
        let corner = size * 0.075
        ZStack {
            // ombre portée sous le disque, c elle qui le fait flotter au dessus de la tuile
            RoundedRectangle(cornerRadius: corner, style: .continuous)
                .fill(Color(red: 0.16, green: 0.08, blue: 0.4).opacity(0.45))
                .frame(width: width * 0.92, height: height)
                .blur(radius: size * 0.035)
                .offset(y: size * 0.05)
            RoundedRectangle(cornerRadius: corner, style: .continuous)
                .fill(LinearGradient(
                    colors: [.white, Color(red: 0.9, green: 0.89, blue: 1)],
                    startPoint: .top,
                    endPoint: .bottom
                ))
                .frame(width: width, height: height)
            // tranche du bas un peu plus sombre pr donner l'épaisseur
            RoundedRectangle(cornerRadius: corner, style: .continuous)
                .fill(LinearGradient(
                    colors: [.clear, Color(red: 0.55, green: 0.5, blue: 0.95).opacity(0.35)],
                    startPoint: UnitPoint(x: 0.5, y: 0.62),
                    endPoint: .bottom
                ))
                .frame(width: width, height: height)
            RoundedRectangle(cornerRadius: corner, style: .continuous)
                .strokeBorder(.white, lineWidth: max(size * 0.006, 0.5))
                .frame(width: width, height: height)
            // la fente et le voyant, juste assez pr qu'on reconnaisse un disque
            Capsule()
                .fill(Color(red: 0.62, green: 0.58, blue: 0.95).opacity(0.55))
                .frame(width: width * 0.42, height: size * 0.026)
                .offset(x: -width * 0.16, y: height * 0.12)
            Circle()
                .fill(Color(red: 0.2, green: 0.85, blue: 0.62))
                .frame(width: size * 0.042, height: size * 0.042)
                .shadow(color: Color(red: 0.2, green: 0.85, blue: 0.62).opacity(0.9), radius: size * 0.012)
                .offset(x: width * 0.3, y: height * 0.12)
        }
    }
}

/// The mark alone, in the foreground colour, without the tile: for the
/// menu bar, where icons are a single colour.
struct LevraGlyph: View {
    var size: CGFloat

    var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: size * 0.16, style: .continuous)
                .stroke(lineWidth: size * 0.1)
                .frame(width: size * 0.78, height: size * 0.44)
                .offset(x: -size * 0.08, y: size * 0.2)
            Capsule()
                .frame(width: size * 0.26, height: size * 0.08)
                .offset(x: -size * 0.2, y: size * 0.24)
            Sparkle()
                .frame(width: size * 0.38, height: size * 0.38)
                .offset(x: size * 0.31, y: -size * 0.27)
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }
}

/// A four-pointed star with curved sides.
private struct Sparkle: Shape {
    func path(in rect: CGRect) -> Path {
        let c = CGPoint(x: rect.midX, y: rect.midY)
        let rx = rect.width / 2
        let ry = rect.height / 2
        // les côtés creusés vers le centre, plus doux qu'une étoile à angles droits
        let pinch: CGFloat = 0.18
        var path = Path()
        path.move(to: CGPoint(x: c.x, y: c.y - ry))
        path.addQuadCurve(to: CGPoint(x: c.x + rx, y: c.y), control: CGPoint(x: c.x + rx * pinch, y: c.y - ry * pinch))
        path.addQuadCurve(to: CGPoint(x: c.x, y: c.y + ry), control: CGPoint(x: c.x + rx * pinch, y: c.y + ry * pinch))
        path.addQuadCurve(to: CGPoint(x: c.x - rx, y: c.y), control: CGPoint(x: c.x - rx * pinch, y: c.y + ry * pinch))
        path.addQuadCurve(to: CGPoint(x: c.x, y: c.y - ry), control: CGPoint(x: c.x - rx * pinch, y: c.y - ry * pinch))
        path.closeSubpath()
        return path
    }
}
