import Foundation

struct DiskSpace: Equatable {
    /// What the Finder shows as available, purgeable space included.
    let available: Int64
    let total: Int64

    var used: Double {
        total > 0 ? Double(total - available) / Double(total) : 0
    }

    static func current() -> DiskSpace? {
        let home = URL(fileURLWithPath: NSHomeDirectory())
        let keys: Set<URLResourceKey> = [.volumeAvailableCapacityForImportantUsageKey, .volumeTotalCapacityKey]
        guard let values = try? home.resourceValues(forKeys: keys),
              let available = values.volumeAvailableCapacityForImportantUsage,
              let total = values.volumeTotalCapacity
        else { return nil }
        return DiskSpace(available: available, total: Int64(total))
    }
}
