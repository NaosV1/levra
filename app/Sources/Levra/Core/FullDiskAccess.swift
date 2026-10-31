import Foundation

/// Whether macOS gave Levra Full Disk Access. There is no API to ask:
/// the answer is whether folders it protects can be listed.
enum FullDiskAccess {
    /// Relative to the home folder; the first one that exists tells.
    private static let probes = [".Trash", "Library/Safari", "Library/Mail", "Library/Messages"]

    static var isGranted: Bool {
        let home = URL(fileURLWithPath: NSHomeDirectory())
        for probe in probes {
            do {
                _ = try FileManager.default.contentsOfDirectory(atPath: home.appending(path: probe).path)
                return true
            } catch let error as NSError where error.code == NSFileReadNoSuchFileError {
                continue
            } catch {
                return false
            }
        }
        return false
    }
}
