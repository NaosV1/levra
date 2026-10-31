import DebarrasBindings
import SwiftUI

/// The end of an uninstall: a clear verdict, what went to the Trash by
/// kind, and what stayed in place.
struct UninstallResultView: View {
    let plan: AppsModel.Plan
    let outcome: CleanupOutcome

    var body: some View {
        VStack(spacing: 28) {
            VStack(spacing: 16) {
                ResultMark(verdict: verdict)
                VStack(spacing: 6) {
                    Text(title)
                        .font(.system(size: 20, weight: .semibold))
                        .tracking(-0.4)
                        .foregroundStyle(Theme.textPrimary)
                    Text(summary).textStyle(.secondary).monospacedDigit()
                }
                .multilineTextAlignment(.center)
                .accessibilityElement(children: .combine)
            }
            .padding(.top, 12)

            if !removed.isEmpty {
                removedList
            }
            if !outcome.failures.isEmpty {
                failureList
            }
            if outcome.completed > 0 {
                Text("Tout est dans la corbeille : rien n'est perdu tant que tu ne l'as pas vidée.")
                    .textStyle(.caption)
            }
        }
        .frame(maxWidth: .infinity)
    }

    private var verdict: Verdict {
        if outcome.failures.isEmpty { return .done }
        return outcome.completed > 0 ? .partial : .failed
    }

    private var appRemoved: Bool {
        plan.items.contains { $0.kind == .application && outcome.handled.contains($0.id) }
    }

    private var title: String {
        switch verdict {
        case .done: appRemoved ? "\(plan.app.name) est désinstallée" : "Les restes de \(plan.app.name) sont partis"
        case .partial: "\(plan.app.name) est presque désinstallée"
        case .failed: "Rien n'a pu être retiré"
        }
    }

    private var summary: String {
        var parts = [
            "\(Format.bytes(outcome.freedBytes)) libérés",
            "\(outcome.completed) élément(s) mis à la corbeille",
        ]
        if outcome.authorized > 0 {
            parts.append("dont \(outcome.authorized) avec ton autorisation")
        }
        if !outcome.failures.isEmpty {
            parts.append("\(outcome.failures.count) resté(s) en place")
        }
        return parts.joined(separator: " · ")
    }

    /// The handled items, by kind, in the order of the review.
    private var removed: [(kind: LeftoverKind, count: Int, bytes: UInt64)] {
        LeftoverKind.ordered.compactMap { kind in
            let items = plan.items.filter { $0.kind == kind && outcome.handled.contains($0.id) }
            guard !items.isEmpty else { return nil }
            return (kind, items.count, items.reduce(0) { $0 + $1.bytes })
        }
    }

    private var removedList: some View {
        VStack(alignment: .leading, spacing: 8) {
            SectionHeader(title: "Mis à la corbeille", icon: .delete, trailing: Format.bytes(outcome.freedBytes))
            VStack(spacing: 0) {
                ForEach(Array(removed.enumerated()), id: \.element.kind) { index, group in
                    HStack(spacing: 10) {
                        Icon(.check, size: 14, weight: 1.3).foregroundStyle(Theme.success)
                        Text(group.kind.label).textStyle(.body)
                        Text("\(group.count)").textStyle(.caption).monospacedDigit()
                        Spacer()
                        Text(Format.bytes(group.bytes)).textStyle(.bodyStrong).monospacedDigit()
                    }
                    .padding(.horizontal, 14)
                    .padding(.vertical, 10)
                    .overlay(alignment: .bottom) {
                        if index < removed.count - 1 {
                            Rectangle().fill(Theme.border).frame(height: 1).padding(.leading, 14)
                        }
                    }
                    .staggeredAppear(index: index + 2)
                }
            }
            .card()
        }
    }

    private var failureList: some View {
        VStack(alignment: .leading, spacing: 8) {
            SectionHeader(title: "Restés en place", icon: .alert, trailing: "\(outcome.failures.count)")
            VStack(alignment: .leading, spacing: 10) {
                ForEach(outcome.failures, id: \.target) { failure in
                    VStack(alignment: .leading, spacing: 2) {
                        Text(failure.target.abbreviatingHome).textStyle(.mono).lineLimit(1).truncationMode(.middle)
                        Text(failure.reason).textStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
            .padding(14)
            .frame(maxWidth: .infinity, alignment: .leading)
            .card()
        }
    }
}

private enum Verdict {
    case done, partial, failed

    var tint: Color {
        switch self {
        case .done: Theme.success
        case .partial: Theme.warning
        case .failed: Theme.danger
        }
    }
}

/// The large mark of the verdict. Finishing an uninstall is rare, so it
/// gets a little more motion than the rest of the app: the badge settles
/// with a slight bounce, then the check draws itself. With Reduce Motion it
/// only fades in.
private struct ResultMark: View {
    let verdict: Verdict
    @State private var appeared = false
    @State private var drawn = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        ZStack {
            Circle().fill(verdict.tint.opacity(0.12)).frame(width: 112, height: 112)
            Circle().fill(verdict.tint).frame(width: 80, height: 80)
            if verdict == .done {
                CheckShape()
                    .trim(from: 0, to: drawn ? 1 : 0)
                    .stroke(.white, style: StrokeStyle(lineWidth: 5.5, lineCap: .round, lineJoin: .round))
                    .frame(width: 38, height: 38)
            } else {
                Icon(.alert, size: 38, weight: 1.6).foregroundStyle(.white)
            }
        }
        .frame(width: 112, height: 112)
        .scaleEffect(appeared || reduceMotion ? 1 : 0.8)
        .opacity(appeared ? 1 : 0)
        .accessibilityHidden(true)
        .onAppear {
            if reduceMotion {
                drawn = true
                withAnimation(Motion.snappy) { appeared = true }
            } else {
                withAnimation(.spring(duration: 0.45, bounce: 0.25)) { appeared = true }
                withAnimation(Motion.easeOut(0.35).delay(0.15)) { drawn = true }
            }
        }
    }
}

private struct CheckShape: Shape {
    func path(in rect: CGRect) -> Path {
        var path = Path()
        path.move(to: CGPoint(x: rect.minX + rect.width * 0.14, y: rect.minY + rect.height * 0.54))
        path.addLine(to: CGPoint(x: rect.minX + rect.width * 0.40, y: rect.minY + rect.height * 0.80))
        path.addLine(to: CGPoint(x: rect.minX + rect.width * 0.88, y: rect.minY + rect.height * 0.24))
        return path
    }
}
