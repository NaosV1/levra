import Charts
import SwiftUI

/// The space used over the last 30 days, and the folders that took the
/// most of it over the last week.
struct HistoryCard: View {
    let history: DiskHistory
    @State private var hovered: DiskHistory.Sample?

    var body: some View {
        let samples = history.recent(days: 30)
        VStack(alignment: .leading, spacing: 16) {
            HStack(spacing: 8) {
                Icon(.activity, size: 16).foregroundStyle(Theme.textSecondary)
                Text("Évolution sur 30 jours").textStyle(.label)
                Spacer()
                if let change = change(in: samples) {
                    Badge(
                        text: "\(change >= 0 ? "+" : "−")\(Format.bytes(UInt64(abs(change))))",
                        tone: change > 5_000_000_000 ? .warning : change < 0 ? .success : .neutral
                    )
                }
            }
            if samples.count < 2 {
                Text("Levra mesure le disque une fois par jour : la courbe se dessine au fil des jours.")
                    .textStyle(.secondary)
                    .frame(maxWidth: .infinity, minHeight: 120, alignment: .center)
            } else {
                chart(samples)
            }
            growth
        }
        .padding(18)
        .card()
    }

    private func chart(_ samples: [DiskHistory.Sample]) -> some View {
        let values = samples.map(\.used)
        let low = Double(values.min() ?? 0)
        let high = Double(values.max() ?? 0)
        // on zoome sur la plage utile sinon la courbe est toute plate en haut
        let margin = max((high - low) * 0.3, 1_000_000_000)
        return Chart(samples, id: \.date) { sample in
            AreaMark(
                x: .value("Jour", sample.date, unit: .day),
                yStart: .value("Base", low - margin),
                yEnd: .value("Utilisé", Double(sample.used))
            )
            .foregroundStyle(
                LinearGradient(colors: [Theme.accent.opacity(0.22), Theme.accent.opacity(0.02)], startPoint: .top, endPoint: .bottom)
            )
            .interpolationMethod(.monotone)
            LineMark(x: .value("Jour", sample.date, unit: .day), y: .value("Utilisé", Double(sample.used)))
                .foregroundStyle(Theme.accent)
                .lineStyle(StrokeStyle(lineWidth: 2, lineCap: .round))
                .interpolationMethod(.monotone)
            if let hovered, hovered.date == sample.date {
                PointMark(x: .value("Jour", sample.date, unit: .day), y: .value("Utilisé", Double(sample.used)))
                    .foregroundStyle(Theme.accent)
                    .annotation(position: .top, spacing: 6) {
                        VStack(spacing: 1) {
                            Text(Format.bytes(UInt64(max(sample.used, 0)))).textStyle(.bodyStrong).monospacedDigit()
                            Text(sample.date.formatted(.dateTime.day().month())).textStyle(.caption)
                        }
                        .padding(.horizontal, 8)
                        .padding(.vertical, 4)
                        .background(Theme.surface, in: RoundedRectangle(cornerRadius: Theme.radiusSmall))
                        .overlay(RoundedRectangle(cornerRadius: Theme.radiusSmall).strokeBorder(Theme.border))
                    }
            }
        }
        .chartYScale(domain: (low - margin)...(high + margin))
        .chartYAxis {
            AxisMarks(position: .trailing, values: .automatic(desiredCount: 3)) { value in
                AxisGridLine().foregroundStyle(Theme.border)
                AxisValueLabel {
                    if let bytes = value.as(Double.self) {
                        Text(Format.bytes(UInt64(max(bytes, 0)))).textStyle(.caption)
                    }
                }
            }
        }
        .chartXAxis {
            AxisMarks(values: .stride(by: .day, count: 7)) { _ in
                AxisValueLabel(format: .dateTime.day().month(), centered: true).foregroundStyle(Theme.textTertiary)
            }
        }
        .chartOverlay { proxy in
            Rectangle().fill(.clear).contentShape(Rectangle())
                .onContinuousHover { phase in
                    switch phase {
                    case let .active(location):
                        guard let date: Date = proxy.value(atX: location.x) else { return }
                        hovered = samples.min { abs($0.date.timeIntervalSince(date)) < abs($1.date.timeIntervalSince(date)) }
                    case .ended:
                        hovered = nil
                    }
                }
        }
        .frame(height: 140)
        .animation(Motion.snappy, value: samples.count)
    }

    @ViewBuilder
    private var growth: some View {
        let grown = history.largestGrowth(days: 7)
        if !grown.isEmpty {
            VStack(alignment: .leading, spacing: 6) {
                Text("Ce qui a grossi cette semaine").textStyle(.label)
                VStack(spacing: 0) {
                    ForEach(Array(grown.enumerated()), id: \.element.id) { index, item in
                        CardRow(isLast: index == grown.count - 1) {
                            HStack(spacing: 10) {
                                Icon(.folder, size: 14).foregroundStyle(Theme.textSecondary)
                                Text(item.path.abbreviatingHome).textStyle(.body).lineLimit(1).truncationMode(.middle)
                                Spacer(minLength: 8)
                                HoverReveal {
                                    IconButton(icon: .arrowUpRight, help: "Afficher dans le Finder") { SystemTrash.reveal(item.path) }
                                }
                                Text("+\(Format.bytes(UInt64(max(item.grown, 0))))")
                                    .textStyle(.bodyStrong)
                                    .monospacedDigit()
                                    .foregroundStyle(item.grown > 5_000_000_000 ? Theme.warning : Theme.textPrimary)
                            }
                        }
                    }
                }
                .card()
            }
        }
    }

    /// Change over the period shown, `nil` with a single measurement.
    private func change(in samples: [DiskHistory.Sample]) -> Int64? {
        guard samples.count >= 2, let first = samples.first, let last = samples.last else { return nil }
        return last.used - first.used
    }
}
