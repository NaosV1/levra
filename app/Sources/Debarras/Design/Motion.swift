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
