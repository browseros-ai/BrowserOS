import Cocoa
import SwiftUI

/// Manages the side panel window lifecycle, positioning, and animation.
final class WindowManager {
    private(set) var panel: NSWindow?
    private var hostingController: NSHostingController<AnyView>?
    private let screenManager = TriosScreenManager.shared
    private var currentSidebarWidth: CGFloat = 400
    private let defaultWidth: CGFloat = 400
    var onPanelToggle: ((Bool) -> Void)?
    static weak var inputFirstResponder: NSView?

    func setupPanel(contentView: AnyView) -> NSWindow {
        guard let screen = NSScreen.main else {
            fatalError("No main screen available")
        }
        let frame = screen.visibleFrame
        let panel = KeyWindow(
            contentRect: NSRect(x: frame.maxX - defaultWidth, y: frame.minY, width: defaultWidth, height: frame.height),
            styleMask: [.titled, .fullSizeContentView, .resizable, .closable, .miniaturizable],
            backing: .buffered,
            defer: false
        )
        panel.level = .floating
        panel.hidesOnDeactivate = false
        panel.isMovableByWindowBackground = true
        panel.backgroundColor = .clear
        panel.isOpaque = false
        panel.hasShadow = true
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .transient, .ignoresCycle]
        panel.animationBehavior = .none
        panel.appearance = NSAppearance(named: .darkAqua)
        panel.isReleasedWhenClosed = false
        panel.titlebarAppearsTransparent = true
        panel.titleVisibility = .hidden

        // Glassmorphism blur - subview behind everything
        let blurView = NSVisualEffectView()
        blurView.material = .fullScreenUI
        blurView.blendingMode = .behindWindow
        blurView.state = .active
        blurView.wantsLayer = true
        blurView.frame = panel.contentView!.bounds
        blurView.autoresizingMask = [.width, .height]
        panel.contentView?.addSubview(blurView, positioned: .below, relativeTo: nil)

        let hc = NSHostingController(rootView: contentView)
        let hostingView = hc.view
        hostingView.frame = panel.contentView!.bounds
        hostingView.autoresizingMask = [.width, .height]
        panel.contentView?.addSubview(hostingView)

        self.hostingController = hc
        self.panel = panel
        screenManager.currentScreen = screen
        panel.makeKeyAndOrderFront(nil)
        NSApplication.shared.activate(ignoringOtherApps: true)
        return panel
    }

    func toggle() {
        guard let panel = panel else { return }
        panel.isVisible ? close() : open()
    }

    func open() {
        NSLog("WindowManager.open() called")
        guard let panel = panel else {
            NSLog("WindowManager.open(): panel is nil!")
            return
        }
        guard let screen = screenManager.detectScreenForMouse() ?? NSScreen.main ?? NSScreen.screens.first else { return }
        let frame = screen.visibleFrame
        let offscreenX = frame.maxX
        let openX = frame.maxX - currentSidebarWidth
        let y = frame.minY
        NSLog("WindowManager.open(): screen frame=\(frame), offscreenX=\(offscreenX), openX=\(openX)")
        panel.setFrame(NSRect(x: offscreenX, y: y, width: currentSidebarWidth, height: frame.height), display: false)
        panel.makeKeyAndOrderFront(nil)
        NSApplication.shared.activate(ignoringOtherApps: true)
        NSLog("WindowManager.open(): makeKeyAndOrderFront called, panel.isVisible=\(panel.isVisible)")

        DispatchQueue.main.asyncAfter(deadline: .now() + 0.15) {
            if let input = WindowManager.inputFirstResponder {
                panel.makeFirstResponder(input)
            }
        }

        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0.35
            context.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
            panel.animator().setFrame(NSRect(x: openX, y: y, width: currentSidebarWidth, height: frame.height), display: true)
        }
        onPanelToggle?(true)
        NSLog("WindowManager.open(): animation done")
    }

    func close(completion: (() -> Void)? = nil) {
        guard let panel = panel else { return }
        let screen = screenManager.currentScreen ?? NSScreen.main
        let frame = screen?.visibleFrame ?? NSScreen.main?.visibleFrame ?? CGRect(x: 0, y: 0, width: 1512, height: 982)
        let offscreenX = frame.maxX
        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0.35
            context.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
            panel.animator().setFrame(NSRect(x: offscreenX, y: panel.frame.origin.y, width: currentSidebarWidth, height: panel.frame.height), display: true)
        } completionHandler: {
            panel.orderOut(nil)
            completion?()
            self.onPanelToggle?(false)
        }
    }

    func resize(to width: CGFloat) {
        guard let panel = panel, width != currentSidebarWidth else { return }
        currentSidebarWidth = width
        let screen = screenManager.currentScreen ?? NSScreen.main
        let frame = screen?.visibleFrame ?? CGRect(x: 0, y: 0, width: 1512, height: 982)
        let x = frame.maxX - width
        let y = frame.minY
        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0.25
            context.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
            panel.animator().setFrame(NSRect(x: x, y: y, width: width, height: frame.height), display: true)
        } completionHandler: {
            NSLog("Panel resized to \(width)")
        }
    }
}
