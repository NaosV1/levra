import Foundation
import LevraBindings
import Observation

/// One measurement a day: the space used on the disk and the size of the
/// home folder's main folders, kept 90 days, to show how it evolves and to
/// warn when a folder suddenly grows.
@MainActor
@Observable
final class DiskHistory {
    struct Sample: Codable, Equatable {
        let date: Date
        let used: Int64
        let total: Int64
        /// Size of each watched folder, by path.
        let folders: [String: UInt64]
    }

    struct Growth: Identifiable, Equatable {
        let path: String
        let from: UInt64
        let to: UInt64
        var id: String { path }
        var grown: Int64 { Int64(to) - Int64(from) }
    }

    private(set) var samples: [Sample] = []
    private(set) var measuring = false

    private static let kept: TimeInterval = 90 * 86_400
    /// A folder taking this much more since the day before is worth a word.
    private static let alertBytes: Int64 = 5_000_000_000
    private static let alertBytesWithRatio: Int64 = 2_000_000_000

    private let file: URL

    init() {
        let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appending(path: "Levra", directoryHint: .isDirectory)
        file = support.appending(path: "history.json")
        if let data = try? Data(contentsOf: file),
           let decoded = try? JSONDecoder().decode([Sample].self, from: data) {
            samples = decoded
        }
    }

    /// Whether today's measurement is still to take.
    var isDue: Bool {
        guard let last = samples.last else { return true }
        return !Calendar.current.isDateInToday(last.date)
    }

    /// Takes today's measurement if not done yet; returns the folders that
    /// grew suddenly since the previous one.
    func recordIfDue() async -> [Growth] {
        guard isDue, !measuring, let disk = DiskSpace.current() else { return [] }
        measuring = true
        defer { measuring = false }
        let folders = Self.watchedFolders()
        // ça mesure presque tt le dossier perso mais c 1 fois par jour et en tâche de fond tkt
        let sizes = await Task.detached(priority: .utility) { measureFolders(paths: folders) }.value
        let sample = Sample(
            date: .now,
            used: disk.total - disk.available,
            total: disk.total,
            folders: Dictionary(uniqueKeysWithValues: zip(folders, sizes))
        )
        let previous = samples.last
        samples.append(sample)
        samples.removeAll { $0.date < Date.now.addingTimeInterval(-Self.kept) }
        save()

        // seulement si la mesure d'avant est récente, sinon c pas "d'un coup"
        guard let previous, sample.date.timeIntervalSince(previous.date) < 3 * 86_400 else { return [] }
        return Self.growth(from: previous, to: sample).filter { growth in
            growth.grown >= Self.alertBytes
                || growth.grown >= Self.alertBytesWithRatio && Double(growth.to) >= Double(growth.from) * 1.5
        }
    }

    /// The folders that grew the most over the last `days` days.
    func largestGrowth(days: Int, limit: Int = 3) -> [Growth] {
        guard let latest = samples.last else { return [] }
        let since = latest.date.addingTimeInterval(-Double(days) * 86_400)
        guard let base = samples.first(where: { $0.date >= since && $0 != latest }) else { return [] }
        return Array(Self.growth(from: base, to: latest).filter { $0.grown > 100_000_000 }.prefix(limit))
    }

    /// Daily samples of the last `days` days, for the chart.
    func recent(days: Int) -> [Sample] {
        let since = Date.now.addingTimeInterval(-Double(days) * 86_400)
        return samples.filter { $0.date >= since }
    }

    private static func growth(from old: Sample, to new: Sample) -> [Growth] {
        new.folders.compactMap { path, size in
            guard let before = old.folders[path], size > before else { return nil }
            return Growth(path: path, from: before, to: size)
        }
        .sorted { $0.grown > $1.grown }
    }

    /// The home folder's own folders and those of its Library, where
    /// caches and logs pile up.
    nonisolated static func watchedFolders() -> [String] {
        let home = URL(fileURLWithPath: NSHomeDirectory())
        let library = home.appending(path: "Library")
        func folders(in dir: URL) -> [String] {
            let keys: [URLResourceKey] = [.isDirectoryKey, .isSymbolicLinkKey]
            let entries = (try? FileManager.default.contentsOfDirectory(at: dir, includingPropertiesForKeys: keys)) ?? []
            return entries.compactMap { url in
                let values = try? url.resourceValues(forKeys: Set(keys))
                guard values?.isDirectory == true, values?.isSymbolicLink != true else { return nil }
                return url.path
            }
        }
        return folders(in: home).filter { $0 != library.path } + folders(in: library)
    }

    private func save() {
        do {
            try FileManager.default.createDirectory(at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
            try JSONEncoder().encode(samples).write(to: file, options: .atomic)
        } catch {
            // pas grave si ça rate, on retentera demain
        }
    }
}
