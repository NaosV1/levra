import SwiftUI

/// Motion rules: short, ease-out, never on keyboard-driven changes, and
/// opacity only when the user asked macOS to reduce motion.
enum Motion {
    /// Strong ease-out: moves at once, lands softly.
    static func easeOut(_ duration: Double) -> Animation {
        .timingCurve(0.23, 1, 0.32, 1, duration: duration)
    }

    /// Press feedback and checkmarks.
    static let quick = easeOut(0.12)
    /// Banners, rows, small state changes.
    static let snappy = easeOut(0.2)
    /// Page changes and the sidebar selection, which moves on screen.
    static let glide = Animation.spring(duration: 0.3, bounce: 0)
    static let hover = Animation.easeOut(duration: 0.12)
}

/// Entering content rises a few points out of a slight blur; the blur
/// hides the crossfade between the old and new state.
private struct Lift: ViewModifier {
    let progress: Double

    func body(content: Content) -> some View {
        content
            .opacity(progress)
            .offset(y: (1 - progress) * 6)
            .blur(radius: (1 - progress) * 2)
    }
}

extension AnyTransition {
    /// Rises in, fades out faster than it came in.
    static func lift(reduceMotion: Bool) -> AnyTransition {
        let insertion: AnyTransition = reduceMotion
            ? .opacity
            : .modifier(active: Lift(progress: 0), identity: Lift(progress: 1))
        return .asymmetric(insertion: insertion, removal: .opacity.animation(Motion.quick))
    }
}

/// Rows of a freshly shown list cascade in. Only the first screenful is
/// staggered: rows scrolled into view later are simply there.
private struct StaggeredAppear: ViewModifier {
    let index: Int
    @State private var visible: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private static let staggered = 12

    init(index: Int) {
        self.index = index
        _visible = State(initialValue: index >= Self.staggered)
    }

    func body(content: Content) -> some View {
        content
            .opacity(visible ? 1 : 0)
            .offset(y: visible || reduceMotion ? 0 : 6)
            .onAppear {
                guard !visible else { return }
                withAnimation(Motion.snappy.delay(Double(index) * 0.03)) { visible = true }
            }
    }
}

extension View {
    func staggeredAppear(index: Int) -> some View {
        modifier(StaggeredAppear(index: index))
    }
}

/// A soft highlight sweeping across a row being worked on. With Reduce
/// Motion it becomes a still tint.
private struct Shimmer: ViewModifier {
    let active: Bool
    @State private var phase: CGFloat = -1
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    func body(content: Content) -> some View {
        content.overlay {
            if active {
                GeometryReader { proxy in
                    if reduceMotion {
                        Theme.accent.opacity(0.06)
                    } else {
                        // une bande de lumière qui traverse la ligne en boucle, discret mais on voit que ça bosse
                        LinearGradient(
                            colors: [.clear, Theme.accent.opacity(0.12), .clear],
                            startPoint: .leading,
                            endPoint: .trailing
                        )
                        .frame(width: proxy.size.width * 0.45)
                        .offset(x: phase * proxy.size.width)
                        .onAppear {
                            phase = -0.45
                            withAnimation(.easeInOut(duration: 1.1).repeatForever(autoreverses: false)) { phase = 1 }
                        }
                    }
                }
                .allowsHitTesting(false)
                .clipped()
                .transition(.opacity)
            }
        }
    }
}

extension View {
    func shimmering(_ active: Bool) -> some View {
        modifier(Shimmer(active: active))
    }
}

extension AnyTransition {
    /// A row done with: slides a few points aside as it fades.
    static func dismissRow(reduceMotion: Bool) -> AnyTransition {
        .asymmetric(
            insertion: .identity,
            removal: reduceMotion ? .opacity : .opacity.combined(with: .offset(x: 24))
        )
    }
}
