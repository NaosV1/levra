import SwiftUI

/// How full the disk is: a ring, with the share used in the middle.
struct DiskRing: View {
    let disk: DiskSpace
    var diameter: CGFloat = 112

    var body: some View {
        ZStack {
            Circle().stroke(Theme.border, lineWidth: lineWidth)
            Circle()
                .trim(from: 0, to: disk.used)
                .stroke(tint, style: StrokeStyle(lineWidth: lineWidth, lineCap: .round))
                .rotationEffect(.degrees(-90))
            VStack(spacing: 1) {
                Text("\(Int((disk.used * 100).rounded())) %")
                    .font(.system(size: diameter * 0.18, weight: .semibold).monospacedDigit())
                    .foregroundStyle(Theme.textPrimary)
                Text("utilisé")
                    .font(.system(size: max(9, diameter * 0.1)))
                    .foregroundStyle(Theme.textSecondary)
            }
        }
        .frame(width: diameter, height: diameter)
        .animation(Motion.snappy, value: disk.used)
    }

    private var lineWidth: CGFloat { diameter * 0.09 }

    /// The accent while there is room, then the colour of the warning.
    private var tint: Color {
        disk.tone == .success ? Theme.accent : disk.tone.color
    }
}
