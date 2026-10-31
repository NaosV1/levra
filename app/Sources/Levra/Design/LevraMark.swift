import SwiftUI

/// Levra's mark: an open square whose last stroke flies off, on a dark
/// tile. Drawn here once, for the app (sidebar, welcome) and for the app
/// icon, which scripts/make-icon.swift renders from this view. Nothing else
/// of the app is used here, so that the script can compile this file alone.
struct LevraMark: View {
    var size: CGFloat

    var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: size * 0.225, style: .continuous)
                .fill(LinearGradient(colors: [Self.tileTop, Self.tileBottom], startPoint: .top, endPoint: .bottom))
            OpenSquare()
                .stroke(Self.stroke, style: StrokeStyle(lineWidth: size * 0.055, lineCap: .round, lineJoin: .round))
            FlyingStroke()
                .stroke(Self.accent, style: StrokeStyle(lineWidth: size * 0.055, lineCap: .round))
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }

    private static let tileTop = Color(red: 0x1C / 255, green: 0x1C / 255, blue: 0x20 / 255)
    private static let tileBottom = Color(red: 0x0E / 255, green: 0x0E / 255, blue: 0x10 / 255)
    private static let stroke = Color(red: 0xED / 255, green: 0xED / 255, blue: 0xEF / 255)
    private static let accent = Color(red: 0x7C / 255, green: 0x86 / 255, blue: 0xE2 / 255)
}

/// On a 100-unit grid: a rounded square open at its top-right corner.
private struct OpenSquare: Shape {
    func path(in rect: CGRect) -> Path {
        let point = gridPoint(in: rect)
        var path = Path()
        path.move(to: point(60, 28))
        path.addLine(to: point(38, 28))
        path.addQuadCurve(to: point(28, 38), control: point(28, 28))
        path.addLine(to: point(28, 62))
        path.addQuadCurve(to: point(38, 72), control: point(28, 72))
        path.addLine(to: point(62, 72))
        path.addQuadCurve(to: point(72, 62), control: point(72, 72))
        path.addLine(to: point(72, 46))
        return path
    }
}

/// The stroke that left the square, past its open corner.
private struct FlyingStroke: Shape {
    func path(in rect: CGRect) -> Path {
        let point = gridPoint(in: rect)
        var path = Path()
        path.move(to: point(69, 31))
        path.addLine(to: point(79, 19))
        return path
    }
}

private func gridPoint(in rect: CGRect) -> (CGFloat, CGFloat) -> CGPoint {
    { x, y in CGPoint(x: rect.minX + rect.width * x / 100, y: rect.minY + rect.height * y / 100) }
}
