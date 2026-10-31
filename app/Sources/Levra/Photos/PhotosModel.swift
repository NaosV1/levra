import Foundation
import ImageIO
import LevraBindings
import Observation

/// Photos that look alike (bursts, retakes, screenshots saved twice,
/// resized copies) in the user's own folders. Each picture gets a 64-bit
/// "difference hash" of a tiny greyscale thumbnail; pictures whose hashes
/// differ by a few bits at most look the same to the eye.
@MainActor
@Observable
final class PhotosModel {
    enum Phase: Equatable {
        case idle
        case scanning(files: UInt64, bytes: UInt64)
        case ready
        case removing
    }

    struct Photo: Identifiable, Hashable, Sendable {
        let path: String
        let bytes: UInt64
        let pixels: Int
        let modified: Date?
        var id: String { path }
    }

    struct Group: Identifiable, Hashable {
        /// Best first: the most pixels, then the largest file.
        var photos: [Photo]
        var id: String { photos.first?.path ?? "" }
        var removableBytes: UInt64 { photos.dropFirst().reduce(0) { $0 + $1.bytes } }
    }

    private(set) var phase: Phase = .idle
    private(set) var groups: [Group] = []
    private(set) var outcome: CleanupOutcome?
    private(set) var error: String?
    var selection: Set<String> = []

    private var progress: HashProgress?

    /// Folders searched, where photos land outside the Photos library.
    nonisolated static let folders = ["Pictures", "Desktop", "Downloads", "Documents", "Movies"]
    nonisolated static let extensions: Set<String> = ["jpg", "jpeg", "png", "heic", "heif", "tiff", "tif", "webp", "gif", "bmp"]
    /// Below this, an icon or a sticker rather than a photo.
    nonisolated static let minBytes: UInt64 = 80_000
    /// Hashes this close are the same picture to the eye.
    nonisolated static let maxDistance = 6

    var selectedBytes: UInt64 {
        groups.flatMap(\.photos).filter { selection.contains($0.path) }.reduce(0) { $0 + $1.bytes }
    }

    var removableBytes: UInt64 {
        groups.reduce(0) { $0 + $1.removableBytes }
    }

    func find() async {
        phase = .scanning(files: 0, bytes: 0)
        error = nil
        outcome = nil
        let home = URL(fileURLWithPath: NSHomeDirectory())
        let roots = Self.folders.map { home.appending(path: $0) }
        let progress = HashProgress()
        self.progress = progress
        let ticker = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(150))
                let (files, bytes) = progress.read()
                guard let self, case .scanning = self.phase else { return }
                self.phase = .scanning(files: files, bytes: bytes)
            }
        }
        let found = await Task.detached(priority: .userInitiated) { () -> [[Photo]] in
            let photos = Self.collect(roots)
            let hashed = Self.hash(photos, progress: progress)
            return Self.cluster(hashed)
        }.value
        ticker.cancel()
        if progress.stopped {
            phase = .idle
            return
        }
        groups = found.map { Group(photos: $0) }.sorted { $0.removableBytes > $1.removableBytes }
        selectProposed()
        phase = .ready
    }

    func cancelScan() {
        progress?.stop()
    }

    /// Every picture except the best of its group.
    func selectProposed() {
        selection = Set(groups.flatMap { $0.photos.dropFirst().map(\.path) })
    }

    /// One picture of a group always stays.
    func setSelected(_ selected: Bool, photo: Photo, in group: Group) {
        guard selected else {
            selection.remove(photo.path)
            return
        }
        guard group.photos.contains(where: { $0.path != photo.path && !selection.contains($0.path) }) else { return }
        selection.insert(photo.path)
    }

    func remove() async {
        guard !selection.isEmpty else { return }
        phase = .removing
        let paths = Array(selection)
        do {
            let result = try await background { try moveToTrash(paths: paths) }
            let outcome = await CleanupOutcome.finishing(result)
            self.outcome = outcome
            groups = groups.compactMap { group in
                var group = group
                group.photos.removeAll { outcome.handled.contains($0.path) }
                return group.photos.count >= 2 ? group : nil
            }
            selection.subtract(outcome.handled)
        } catch {
            self.error = error.message
        }
        phase = .ready
    }

    func dismissOutcome() {
        outcome = nil
    }

    // MARK: Search, away from the main actor

    private nonisolated static func collect(_ roots: [URL]) -> [Photo] {
        let keys: [URLResourceKey] = [
            .isRegularFileKey, .fileAllocatedSizeKey, .contentModificationDateKey, .ubiquitousItemDownloadingStatusKey,
        ]
        var photos: [Photo] = []
        for root in roots {
            guard let enumerator = FileManager.default.enumerator(
                at: root,
                includingPropertiesForKeys: keys,
                options: [.skipsHiddenFiles, .skipsPackageDescendants]
            ) else { continue }
            for case let url as URL in enumerator {
                guard extensions.contains(url.pathExtension.lowercased()),
                      let values = try? url.resourceValues(forKeys: Set(keys)),
                      values.isRegularFile == true
                else { continue }
                // une photo restée ds icloud on la télécharge pas juste pr la comparer
                if let status = values.ubiquitousItemDownloadingStatus, status != .current { continue }
                let bytes = UInt64(values.fileAllocatedSize ?? 0)
                guard bytes >= minBytes else { continue }
                photos.append(Photo(path: url.path, bytes: bytes, pixels: 0, modified: values.contentModificationDate))
            }
        }
        return photos
    }

    private nonisolated static func hash(_ photos: [Photo], progress: HashProgress) -> [(Photo, UInt64)] {
        let results = HashResults(count: photos.count)
        DispatchQueue.concurrentPerform(iterations: photos.count) { index in
            // annulé : les itérations restantes passent direct
            guard !progress.stopped else { return }
            let photo = photos[index]
            if let (hash, pixels) = differenceHash(of: URL(fileURLWithPath: photo.path)) {
                results.set(index, (Photo(path: photo.path, bytes: photo.bytes, pixels: pixels, modified: photo.modified), hash))
            }
            progress.add(bytes: photo.bytes)
        }
        return results.values()
    }

    /// 9 × 8 greyscale thumbnail, each pixel compared to its right neighbour.
    nonisolated static func differenceHash(of url: URL) -> (UInt64, Int)? {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary) else {
            return nil
        }
        let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any]
        let width = properties?[kCGImagePropertyPixelWidth] as? Int ?? 0
        let height = properties?[kCGImagePropertyPixelHeight] as? Int ?? 0
        // la vignette intégrée suffit largement pr un hash 9x8, pas besoin de décoder 48 Mpx
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageIfAbsent: true,
            kCGImageSourceThumbnailMaxPixelSize: 64,
            kCGImageSourceCreateThumbnailWithTransform: true,
        ]
        guard let thumbnail = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else { return nil }
        var pixels = [UInt8](repeating: 0, count: 9 * 8)
        guard let context = CGContext(
            data: &pixels,
            width: 9,
            height: 8,
            bitsPerComponent: 8,
            bytesPerRow: 9,
            space: CGColorSpaceCreateDeviceGray(),
            bitmapInfo: CGImageAlphaInfo.none.rawValue
        ) else { return nil }
        context.interpolationQuality = .medium
        context.draw(thumbnail, in: CGRect(x: 0, y: 0, width: 9, height: 8))
        var hash: UInt64 = 0
        for row in 0..<8 {
            for column in 0..<8 {
                hash <<= 1
                if pixels[row * 9 + column] > pixels[row * 9 + column + 1] { hash |= 1 }
            }
        }
        return (hash, width * height)
    }

    /// Groups pictures whose hashes are close. Two hashes at most 7 bits
    /// apart share at least one of their 8 bytes, so only pictures sharing
    /// a byte at the same place are compared, never every pair.
    nonisolated static func cluster(_ hashed: [(Photo, UInt64)]) -> [[Photo]] {
        var parent = Array(hashed.indices)
        func root(_ index: Int) -> Int {
            var index = index
            while parent[index] != index {
                parent[index] = parent[parent[index]]
                index = parent[index]
            }
            return index
        }
        var buckets: [Int: [Int]] = [:]
        for (index, (_, hash)) in hashed.enumerated() {
            for byte in 0..<8 {
                buckets[byte << 8 | Int((hash >> (byte * 8)) & 0xFF), default: []].append(index)
            }
        }
        for members in buckets.values where members.count > 1 && members.count < 5_000 {
            for a in 0..<members.count {
                for b in (a + 1)..<members.count {
                    let (i, j) = (members[a], members[b])
                    guard (hashed[i].1 ^ hashed[j].1).nonzeroBitCount <= maxDistance else { continue }
                    // une image toute unie a un hash plein de zéros, elle ressemblerait à tt le reste
                    guard hashed[i].1.nonzeroBitCount > 4, hashed[i].1.nonzeroBitCount < 60 else { continue }
                    parent[root(i)] = root(j)
                }
            }
        }
        var groups: [Int: [Photo]] = [:]
        for index in hashed.indices {
            groups[root(index), default: []].append(hashed[index].0)
        }
        return groups.values
            .filter { $0.count >= 2 }
            .map { $0.sorted { ($0.pixels, $0.bytes) > ($1.pixels, $1.bytes) } }
    }
}

/// Counters shared by the hashing threads and the progress ticker.
private final class HashProgress: @unchecked Sendable {
    private let lock = NSLock()
    private var files: UInt64 = 0
    private var bytes: UInt64 = 0
    private var isStopped = false

    var stopped: Bool { lock.withLock { isStopped } }

    func add(bytes: UInt64) {
        lock.withLock {
            files += 1
            self.bytes += bytes
        }
    }

    func stop() { lock.withLock { isStopped = true } }

    func read() -> (UInt64, UInt64) { lock.withLock { (files, bytes) } }
}

private final class HashResults: @unchecked Sendable {
    private let lock = NSLock()
    private var slots: [(PhotosModel.Photo, UInt64)?]

    init(count: Int) { slots = Array(repeating: nil, count: count) }

    func set(_ index: Int, _ value: (PhotosModel.Photo, UInt64)) { lock.withLock { slots[index] = value } }

    func values() -> [(PhotosModel.Photo, UInt64)] { lock.withLock { slots.compactMap { $0 } } }
}
