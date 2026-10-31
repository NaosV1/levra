import SwiftUI

/// Title, one line of context, and the page's actions on the right.
struct PageHeader<Actions: View>: View {
    let title: String
    let subtitle: String
    @ViewBuilder var actions: Actions

    var body: some View {
        HStack(alignment: .bottom, spacing: 16) {
            VStack(alignment: .leading, spacing: 4) {
                Text(title).textStyle(.pageTitle)
                Text(subtitle)
                    .textStyle(.secondary)
                    .contentTransition(.numericText())
                    .lineLimit(1)
            }
            Spacer(minLength: 16)
            HStack(spacing: 8) { actions }
        }
    }
}

/// The scrolling body shared by every screen: header, then content, kept
/// to a readable width.
struct PageScroll<Header: View, Content: View>: View {
    @ViewBuilder var header: Header
    @ViewBuilder var content: Content

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                header
                content
            }
            .padding(.horizontal, Theme.pagePadding)
            .padding(.top, 20)
            .padding(.bottom, Theme.pagePadding)
            .frame(maxWidth: Theme.contentWidth, alignment: .leading)
            .frame(maxWidth: .infinity)
        }
        .scrollContentBackground(.hidden)
    }
}

struct SectionHeader: View {
    let title: String
    var icon: Papicon?
    var trailing: String?

    var body: some View {
        HStack(spacing: 6) {
            if let icon {
                Icon(icon, size: 14).foregroundStyle(Theme.textTertiary)
            }
            Text(title).textStyle(.label)
            Spacer()
            if let trailing {
                Text(trailing)
                    .textStyle(.label)
                    .monospacedDigit()
                    .contentTransition(.numericText())
            }
        }
        .padding(.horizontal, 4)
    }
}

/// A row inside a card: highlights under the pointer, separated from the
/// next by a hairline.
struct CardRow<Content: View>: View {
    var isLast = false
    @ViewBuilder var content: Content
    @State private var hovering = false

    var body: some View {
        content
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(hovering ? Theme.surfaceHover : .clear)
            .overlay(alignment: .bottom) {
                if !isLast {
                    Rectangle().fill(Theme.border).frame(height: 1).padding(.leading, 14)
                }
            }
            .contentShape(Rectangle())
            .onHover { hovering = $0 }
            .animation(Motion.hover, value: hovering)
            .environment(\.isRowHovered, hovering)
    }
}

extension EnvironmentValues {
    /// Lets a row reveal its actions only while the pointer is over it.
    @Entry var isRowHovered = false
}

/// Shows its content while the enclosing row is hovered (and keeps it in
/// the layout, so nothing shifts).
struct HoverReveal<Content: View>: View {
    @ViewBuilder var content: Content
    @Environment(\.isRowHovered) private var hovered

    var body: some View {
        content
            .opacity(hovered ? 1 : 0)
            .allowsHitTesting(hovered)
            .animation(Motion.hover, value: hovered)
    }
}

struct Banner<Actions: View>: View {
    let tone: Badge.Tone
    let icon: Papicon
    let title: String
    var message: String?
    @ViewBuilder var actions: Actions

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Icon(icon, size: 16)
                .foregroundStyle(tone.color)
                .padding(.top, 1)
            VStack(alignment: .leading, spacing: 3) {
                Text(title).textStyle(.bodyStrong)
                if let message {
                    Text(message).textStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer(minLength: 12)
            HStack(spacing: 8) { actions }
        }
        .padding(14)
        .background(tone.color.opacity(0.06), in: RoundedRectangle(cornerRadius: Theme.radius))
        .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(tone.color.opacity(0.22)))
    }
}

extension Banner where Actions == EmptyView {
    init(tone: Badge.Tone, icon: Papicon, title: String, message: String? = nil) {
        self.init(tone: tone, icon: icon, title: title, message: message) { EmptyView() }
    }
}

struct EmptyState<Actions: View>: View {
    let icon: Papicon
    let title: String
    let message: String
    @ViewBuilder var actions: Actions

    var body: some View {
        VStack(spacing: 14) {
            Icon(icon, size: 22)
                .foregroundStyle(Theme.textSecondary)
                .frame(width: 48, height: 48)
                .card()
            VStack(spacing: 6) {
                Text(title).textStyle(.heading)
                Text(message)
                    .textStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: 420)
            }
            HStack(spacing: 8) { actions }.padding(.top, 4)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 56)
    }
}

/// A labelled figure, for summaries above lists.
struct StatTile: View {
    let label: String
    let value: String
    var detail: String?
    var tone: Badge.Tone?

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 6) {
                if let tone {
                    Circle().fill(tone.color).frame(width: 6, height: 6)
                }
                Text(label).textStyle(.label)
            }
            Text(value)
                .font(.system(size: 18, weight: .semibold).monospacedDigit())
                .foregroundStyle(Theme.textPrimary)
                .tracking(-0.3)
                .contentTransition(.numericText())
            // Always laid out, so tiles side by side share one height.
            Text(detail ?? " ").textStyle(.caption).lineLimit(1)
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .card()
    }
}

struct Spinner: View {
    var size: CGFloat = 16
    @State private var turning = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        Icon(.loading, size: size)
            .foregroundStyle(Theme.textSecondary)
            .rotationEffect(.degrees(turning && !reduceMotion ? 360 : 0))
            .opacity(reduceMotion && turning ? 0.5 : 1)
            .animation(.linear(duration: 0.8).repeatForever(autoreverses: false), value: turning)
            .onAppear { turning = true }
            .accessibilityLabel("En cours")
    }
}

/// What a scan is doing while it runs.
struct ScanningState: View {
    let title: String
    let files: UInt64
    let bytes: UInt64
    let onCancel: () -> Void

    var body: some View {
        VStack(spacing: 16) {
            Spinner(size: 22)
            VStack(spacing: 6) {
                Text(title).textStyle(.heading)
                HStack(spacing: 14) {
                    Label { Text("\(Format.count(files)) fichiers") } icon: { Icon(.file, size: 13) }
                    Label { Text("\(Format.bytes(bytes)) parcourus") } icon: { Icon(.hardDrive, size: 13) }
                }
                .textStyle(.secondary)
                .monospacedDigit()
                .contentTransition(.numericText())
                .animation(Motion.snappy, value: files)
            }
            Button("Annuler", action: onCancel).buttonStyle(.app(.ghost))
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 72)
    }
}

/// Share of a whole, as a thin track.
struct SizeBar: View {
    let fraction: Double
    var width: CGFloat = 80
    var tint: Color = Theme.textSecondary

    var body: some View {
        ZStack(alignment: .leading) {
            Capsule().fill(Theme.border)
            Capsule()
                .fill(tint)
                .frame(width: max(2, width * min(max(fraction, 0), 1)))
        }
        .frame(width: width, height: 4)
        .animation(Motion.snappy, value: fraction)
        .accessibilityLabel("\(Int(fraction * 100)) %")
    }
}

/// The selection summary and main action, pinned under the content.
struct ActionBar<Content: View>: View {
    @ViewBuilder var content: Content

    var body: some View {
        HStack(spacing: 10) { content }
            .padding(.horizontal, Theme.pagePadding)
            .padding(.vertical, 12)
            .frame(maxWidth: .infinity)
            .background(.bar)
            .overlay(alignment: .top) { Rectangle().fill(Theme.border).frame(height: 1) }
    }
}

/// Result of a cleanup, an uninstall or a move to the Trash.
struct OutcomeBanner: View {
    let outcome: CleanupOutcome
    let onDismiss: () -> Void
    @State private var showsFailures = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(spacing: 8) {
            Banner(
                tone: outcome.failures.isEmpty ? .success : .warning,
                icon: outcome.failures.isEmpty ? .checkCircle : .alert,
                title: summary,
                message: outcome.failures.isEmpty ? nil : "\(outcome.failures.count) élément(s) non traités."
            ) {
                if !outcome.failures.isEmpty {
                    Button(showsFailures ? "Masquer" : "Détails") {
                        withAnimation(Motion.snappy) { showsFailures.toggle() }
                    }
                    .buttonStyle(.app(.ghost, size: .small))
                }
                IconButton(icon: .cancel, help: "Fermer", action: onDismiss)
            }
            if showsFailures {
                VStack(alignment: .leading, spacing: 8) {
                    ForEach(outcome.failures, id: \.target) { failure in
                        VStack(alignment: .leading, spacing: 2) {
                            Text(failure.target.abbreviatingHome).textStyle(.mono).lineLimit(1).truncationMode(.middle)
                            Text(failure.reason).textStyle(.secondary)
                        }
                    }
                }
                .padding(14)
                .frame(maxWidth: .infinity, alignment: .leading)
                .card()
                .transition(.lift(reduceMotion: reduceMotion))
            }
        }
    }

    private var summary: String {
        var parts = ["\(Format.bytes(outcome.freedBytes)) libérés", "\(outcome.completed) élément(s) traités"]
        if outcome.authorized > 0 {
            parts.append("dont \(outcome.authorized) avec ton autorisation")
        }
        return parts.joined(separator: " · ")
    }
}

struct FullDiskAccessBanner: View {
    let deniedCount: Int

    var body: some View {
        Banner(
            tone: .warning,
            icon: .lock,
            title: "\(deniedCount) dossier(s) protégés non analysés",
            message: "Corbeille, sauvegardes iPhone, Messages, Mail… Donne l'accès complet au disque à Débarras, puis relance."
        ) {
            Button("Ouvrir les réglages") { PrivacyPane.fullDiskAccess.open() }
                .buttonStyle(.app(.secondary, size: .small))
        }
    }
}
