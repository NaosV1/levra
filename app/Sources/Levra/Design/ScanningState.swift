import SwiftUI

/// A scan in progress, given the whole page: a turning ring around the
/// page's icon, the figures in large type, and, for scans made of steps,
/// a checklist that fills in as each one ends.
struct ScanningState: View {
    struct Step: Identifiable {
        let label: String
        let done: Bool
        var id: String { label }
    }

    let title: String
    let icon: Papicon
    let files: UInt64
    let bytes: UInt64
    var steps: [Step] = []
    let onCancel: () -> Void

    var body: some View {
        VStack(spacing: 32) {
            ScanRing(icon: icon)
            Text(title)
                .font(.system(size: 22, weight: .semibold))
                .tracking(-0.5)
                .foregroundStyle(Theme.textPrimary)
            HStack(spacing: 12) {
                Figure(value: Format.count(files), label: "fichiers lus")
                Figure(value: Format.bytes(bytes), label: "lus sur le disque")
            }
            .frame(maxWidth: 460)
            if !steps.isEmpty {
                checklist
            }
            Button("Annuler", action: onCancel).buttonStyle(.app(.secondary))
        }
        .frame(maxWidth: .infinity)
        // The page header sits above: the rest of the window is ours.
        .containerRelativeFrame(.vertical) { height, _ in max(height - 150, 440) }
    }

    private var checklist: some View {
        LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 10), count: 3), spacing: 10) {
            ForEach(steps) { step in
                HStack(spacing: 8) {
                    ZStack {
                        if step.done {
                            Icon(.checkCircle, size: 16, weight: 1.2)
                                .foregroundStyle(Theme.success)
                                .transition(.scale(scale: 0.6).combined(with: .opacity))
                        } else {
                            Circle()
                                .strokeBorder(Theme.borderStrong, style: StrokeStyle(lineWidth: 1.5, dash: [2.5, 2.5]))
                                .frame(width: 14, height: 14)
                        }
                    }
                    .frame(width: 16, height: 16)
                    Text(step.label)
                        .textStyle(step.done ? .body : .secondary)
                        .lineLimit(1)
                    Spacer(minLength: 0)
                }
                .padding(.horizontal, 12)
                .frame(height: 38)
                .card()
                .animation(Motion.snappy, value: step.done)
                .accessibilityElement(children: .combine)
                .accessibilityValue(step.done ? "Terminé" : "En cours")
            }
        }
        .frame(maxWidth: 640)
    }
}

/// A large number with what it counts.
private struct Figure: View {
    let value: String
    let label: String

    var body: some View {
        VStack(spacing: 4) {
            Text(value)
                .textStyle(.metric)
                .contentTransition(.numericText())
                .animation(Motion.snappy, value: value)
            Text(label).textStyle(.caption)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 16)
        .card()
    }
}

/// An arc chasing its tail around the page's icon. It never claims a
/// percentage: steps finish in no fixed order and take very different
/// times. With Reduce Motion the arc stays still.
private struct ScanRing: View {
    let icon: Papicon
    @State private var turning = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        ZStack {
            Circle().stroke(Theme.border, lineWidth: 6)
            Circle()
                .trim(from: 0, to: 0.3)
                .stroke(
                    AngularGradient(
                        colors: [Theme.accent.opacity(0), Theme.accent],
                        center: .center,
                        startAngle: .degrees(0),
                        endAngle: .degrees(108)
                    ),
                    style: StrokeStyle(lineWidth: 6, lineCap: .round)
                )
                .rotationEffect(.degrees(turning ? 360 : 0))
            Icon(icon, size: 36, weight: 1.1).foregroundStyle(Theme.textPrimary)
        }
        .frame(width: 128, height: 128)
        .onAppear {
            guard !reduceMotion else { return }
            withAnimation(.linear(duration: 0.9).repeatForever(autoreverses: false)) { turning = true }
        }
        .accessibilityElement()
        .accessibilityLabel("Analyse en cours")
    }
}

/// A short wait with nothing to count (listing apps, reading memory): the
/// page's ring alone, centred like a scan.
struct PageLoading: View {
    let icon: Papicon
    let label: String

    var body: some View {
        VStack(spacing: 24) {
            ScanRing(icon: icon)
            Text(label).textStyle(.secondary)
        }
        .frame(maxWidth: .infinity)
        .containerRelativeFrame(.vertical) { height, _ in max(height - 150, 380) }
    }
}
