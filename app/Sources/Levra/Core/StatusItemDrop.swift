import AppKit

/// Lets an app be dropped on Levra's menu bar icon to uninstall it.
/// SwiftUI's menu bar extra offers no drop target, so a transparent view
/// is laid over the icon's button: it takes drags, and lets clicks
/// through to the button underneath.
@MainActor
enum StatusItemDrop {
    private static var installed = false

    static func install(onDrop: @escaping @MainActor (URL) -> Void, attempt: Int = 0) {
        guard !installed else { return }
        // la fenêtre de l'icône arrive un peu après le lancement donc on réessaie qq fois
        guard let button = statusButton() else {
            if attempt < 10 {
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
                    install(onDrop: onDrop, attempt: attempt + 1)
                }
            }
            return
        }
        let overlay = DropOverlay(frame: button.bounds, button: button, onDrop: onDrop)
        overlay.autoresizingMask = [.width, .height]
        button.addSubview(overlay)
        installed = true
    }

    private static func statusButton() -> NSStatusBarButton? {
        for window in NSApplication.shared.windows where window.className.contains("StatusBarWindow") {
            if let button = find(in: window.contentView) {
                return button
            }
        }
        return nil
    }

    private static func find(in view: NSView?) -> NSStatusBarButton? {
        guard let view else { return nil }
        if let button = view as? NSStatusBarButton { return button }
        for subview in view.subviews {
            if let button = find(in: subview) { return button }
        }
        return nil
    }
}

private final class DropOverlay: NSView {
    private weak var button: NSStatusBarButton?
    private let onDrop: @MainActor (URL) -> Void

    init(frame: NSRect, button: NSStatusBarButton, onDrop: @escaping @MainActor (URL) -> Void) {
        self.button = button
        self.onDrop = onDrop
        super.init(frame: frame)
        registerForDraggedTypes([.fileURL])
    }

    required init?(coder: NSCoder) { nil }

    /// Clicks go to the button; only drags stop here.
    override func hitTest(_ point: NSPoint) -> NSView? {
        switch NSApplication.shared.currentEvent?.type {
        case .leftMouseDown, .leftMouseUp, .leftMouseDragged, .rightMouseDown, .rightMouseUp, .otherMouseDown, .otherMouseUp, .mouseMoved:
            return nil
        default:
            return super.hitTest(point)
        }
    }

    override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation {
        guard app(in: sender) != nil else { return [] }
        // l'icône s'allume qd on survole avec une app, comme ça on sait que ça va marcher
        button?.highlight(true)
        return .copy
    }

    override func draggingExited(_ sender: NSDraggingInfo?) {
        button?.highlight(false)
    }

    override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
        button?.highlight(false)
        guard let url = app(in: sender) else { return false }
        onDrop(url)
        return true
    }

    private func app(in sender: NSDraggingInfo) -> URL? {
        let urls = sender.draggingPasteboard.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL]
        return urls?.first { $0.pathExtension == "app" }
    }
}
