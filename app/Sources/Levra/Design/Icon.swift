import SwiftUI

/// A PapIcon at any size, in the current foreground style.
struct Icon: View {
    let icon: Papicon
    var size: CGFloat = 16
    /// Multiplies the stroke; 1 matches the original drawing.
    var weight: CGFloat = 1

    init(_ icon: Papicon, size: CGFloat = 16, weight: CGFloat = 1) {
        self.icon = icon
        self.size = size
        self.weight = weight
    }

    var body: some View {
        ZStack {
            ForEach(Array(icon.strokes.enumerated()), id: \.offset) { _, layer in
                IconLayer(path: layer.path)
                    .stroke(style: StrokeStyle(lineWidth: lineWidth(layer.width), lineCap: .round, lineJoin: .round))
            }
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }

    /// The grid's 1.5-unit stroke, scaled, but never under 1.2 pt so small
    /// icons stay crisp; dots keep their proportion to it.
    private func lineWidth(_ gridWidth: CGFloat) -> CGFloat {
        max(1.2, 1.5 * size / 24) * (gridWidth / 1.5) * weight
    }
}

private struct IconLayer: Shape {
    let path: Path

    func path(in rect: CGRect) -> Path {
        path.applying(CGAffineTransform(scaleX: rect.width / 24, y: rect.height / 24))
    }
}
