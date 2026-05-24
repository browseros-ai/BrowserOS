import Cocoa
import Foundation
import SwiftUI
import ApplicationServices

// MARK: - AppDelegate

class AppDelegate: NSObject, NSApplicationDelegate {
    var statusItem: NSStatusItem?
    var sidePanel: NSPanel?
    var hostingController: NSHostingController<ChatPanelView>?
    var serverTask: Process?
    var funnelTask: Process?
    var serverRunning = false
    var funnelRunning = false
    var windowStates: [(AXUIElement, CGRect)] = []
    let sidebarWidth: CGFloat = 400
    let compositionRoot = CompositionRoot()
    var accessibilityGranted = false
    var accessibilityPromptShown = false
    var chatViewModel: ChatViewModel?

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSLog("applicationDidFinishLaunching called")
        let app = NSApplication.shared
        app.setActivationPolicy(.accessory)

        var logoImage: NSImage?
        if let logoURL = Bundle.main.url(forResource: "logo", withExtension: "png"),
           let image = NSImage(contentsOf: logoURL) {
            NSLog("Logo loaded from \(logoURL)")
            logoImage = image
        } else {
            let fallbackPaths = [
                "/Users/playra/BrowserOS-full/trios/logo.png",
                "/Users/playra/BrowserOS-full/trios/trios.app/Contents/Resources/logo.png"
            ]
            for path in fallbackPaths {
                if FileManager.default.fileExists(atPath: path),
                   let image = NSImage(contentsOfFile: path) {
                    NSLog("Logo loaded from fallback \(path)")
                    logoImage = image
                    break
                }
            }
        }

        if let image = logoImage {
            statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
            image.isTemplate = true
            image.size = NSSize(width: 22, height: 22)
            statusItem?.button?.image = image
            statusItem?.button?.imagePosition = .imageOnly
            statusItem?.button?.title = ""
        } else {
            NSLog("Logo not found anywhere, using title fallback")
            statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
            statusItem?.button?.title = "Trios"
        }
        statusItem?.button?.toolTip = "TRIOS AGENT"

        statusItem?.button?.action = #selector(statusBarButtonClicked(_:))
        statusItem?.button?.sendAction(on: [.leftMouseUp, .rightMouseUp])
        statusItem?.menu = nil

        setupSidePanel()
        accessibilityGranted = AXIsProcessTrusted()

        Task {
            await chatViewModel?.registerA2A()
        }
    }

    func setupSidePanel() {
        NSLog("setupSidePanel called")
        guard let screen = NSScreen.main else {
            NSLog("No main screen")
            return
        }
        let screenFrame = screen.visibleFrame
        let panelHeight = screenFrame.height
        let x = screenFrame.maxX
        let y = screenFrame.minY

        let panel = NSPanel(
            contentRect: NSRect(x: x, y: y, width: sidebarWidth, height: panelHeight),
            styleMask: [.titled, .nonactivatingPanel, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )

        panel.isFloatingPanel = true
        panel.level = .mainMenu
        panel.hidesOnDeactivate = false
        panel.becomesKeyOnlyIfNeeded = true
        panel.isMovableByWindowBackground = true
        panel.backgroundColor = NSColor.black
        panel.isOpaque = true
        panel.hasShadow = true
        panel.titlebarAppearsTransparent = true
        panel.titleVisibility = .hidden
        panel.standardWindowButton(.closeButton)?.isHidden = false
        panel.standardWindowButton(.miniaturizeButton)?.isHidden = true
        panel.standardWindowButton(.zoomButton)?.isHidden = true
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .transient]
        panel.animationBehavior = .none
        panel.appearance = NSAppearance(named: .darkAqua)
        panel.isReleasedWhenClosed = false

        let viewModel = compositionRoot.makeChatViewModel()
        self.chatViewModel = viewModel
        let chatView = ChatPanelView(viewModel: viewModel)
        let hc = NSHostingController(rootView: chatView)
        let hostingView = hc.view
        hostingView.frame = panel.contentView!.bounds
        hostingView.autoresizingMask = [.width, .height]
        panel.contentView?.addSubview(hostingView)
        NSLog("SwiftUI hosting view added, frame: \(hostingView.frame)")

        self.hostingController = hc
        self.sidePanel = panel
    }

    // MARK: - Window Shifting

    func getWindowFrame(_ window: AXUIElement) -> CGRect? {
        var positionValue: CFTypeRef?
        guard AXUIElementCopyAttributeValue(window, kAXPositionAttribute as CFString, &positionValue) == .success else {
            return nil
        }
        guard CFGetTypeID(positionValue) == AXValueGetTypeID() else { return nil }
        var position = CGPoint.zero
        guard AXValueGetValue(positionValue as! AXValue, .cgPoint, &position) else { return nil }

        var sizeValue: CFTypeRef?
        guard AXUIElementCopyAttributeValue(window, kAXSizeAttribute as CFString, &sizeValue) == .success else {
            return nil
        }
        guard CFGetTypeID(sizeValue) == AXValueGetTypeID() else { return nil }
        var size = CGSize.zero
        guard AXValueGetValue(sizeValue as! AXValue, .cgSize, &size) else { return nil }

        return CGRect(origin: position, size: size)
    }

    func setWindowFrame(_ window: AXUIElement, frame: CGRect) {
        var position = frame.origin
        var size = frame.size
        if let posValue = AXValueCreate(.cgPoint, &position) {
            AXUIElementSetAttributeValue(window, kAXPositionAttribute as CFString, posValue)
        }
        if let sizeValue = AXValueCreate(.cgSize, &size) {
            AXUIElementSetAttributeValue(window, kAXSizeAttribute as CFString, sizeValue)
        }
    }

    func getAllWindows() -> [(AXUIElement, CGRect)] {
        var result: [(AXUIElement, CGRect)] = []
        let currentPid = getpid()
        guard let screen = NSScreen.main else { return result }
        let screenFrame = screen.frame

        for app in NSWorkspace.shared.runningApplications {
            guard app.activationPolicy == .regular else { continue }
            let pid = app.processIdentifier
            if pid == currentPid { continue }

            let axApp = AXUIElementCreateApplication(pid)
            var windowsValue: CFTypeRef?
            let copyResult = AXUIElementCopyAttributeValue(axApp, kAXWindowsAttribute as CFString, &windowsValue)

            if copyResult == .success, let windowList = windowsValue as? [AXUIElement] {
                for window in windowList {
                    if let frame = getWindowFrame(window) {
                        guard frame.width > 100, frame.height > 100 else { continue }
                        guard frame.intersects(screenFrame) else { continue }
                        result.append((window, frame))
                    }
                }
            }
        }
        return result
    }

    func shiftWindows() {
        guard let screen = NSScreen.main else { return }
        let screenFrame = screen.frame
        let cutoffX = screenFrame.maxX - sidebarWidth
        windowStates.removeAll()
        let allWindows = getAllWindows()
        for (window, frame) in allWindows {
            guard frame.maxX > cutoffX else { continue }
            windowStates.append((window, frame))
            var newFrame = frame
            let overlap = frame.maxX - cutoffX
            let minWidth: CGFloat = 400
            if frame.origin.x >= overlap {
                newFrame.origin.x -= overlap
            } else if frame.width - overlap >= minWidth {
                newFrame.size.width -= overlap
            } else {
                newFrame.origin.x = 0
                newFrame.size.width = cutoffX
            }
            setWindowFrame(window, frame: newFrame)
        }
    }

    func restoreWindows() {
        for (window, frame) in windowStates {
            setWindowFrame(window, frame: frame)
        }
        windowStates.removeAll()
    }

    // MARK: - UI Actions

    @objc func statusBarButtonClicked(_ sender: Any?) {
        NSLog("statusBarButtonClicked called")
        guard let event = NSApp.currentEvent else {
            NSLog("No current event, defaulting to toggle panel")
            toggleSidePanel()
            return
        }
        NSLog("Event type: \(event.type.rawValue)")
        if event.type == .rightMouseUp {
            let menu = createMenu()
            statusItem?.menu = menu
            statusItem?.button?.performClick(nil)
            statusItem?.menu = nil
        } else {
            toggleSidePanel()
        }
    }

    @objc func toggleSidePanel() {
        NSLog("toggleSidePanel called")
        guard let panel = sidePanel else {
            NSLog("sidePanel is nil!")
            return
        }
        NSLog("Panel state — isVisible: \(panel.isVisible), frame: \(panel.frame)")

        if panel.isVisible {
            NSAnimationContext.runAnimationGroup({ context in
                context.duration = 0.35
                context.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
                guard let screen = NSScreen.main else { return }
                let targetX = screen.frame.maxX
                panel.animator().setFrameOrigin(NSPoint(x: targetX, y: panel.frame.origin.y))
            }, completionHandler: {
                panel.orderOut(nil)
                self.restoreWindows()
                NSLog("Panel closed")
            })
        } else {
            accessibilityGranted = AXIsProcessTrusted()
            if accessibilityGranted {
                shiftWindows()
            } else if !accessibilityPromptShown {
                accessibilityPromptShown = true
                showAlert("Please grant Trios Accessibility access in:\nSystem Settings → Privacy & Security → Accessibility")
            }
            guard let screen = NSScreen.main else {
                return
            }
            NSLog("Got screen: \(screen.frame)")
            let screenFrame = screen.visibleFrame
            let panelHeight = screenFrame.height
            let openX = screenFrame.maxX - sidebarWidth - 12
            let offscreenX = screenFrame.maxX
            let y = screenFrame.minY

            panel.setFrame(NSRect(x: offscreenX, y: y, width: sidebarWidth, height: panelHeight), display: false)
            NSLog("Panel positioned off-screen at x=\(offscreenX), about to order front")

            panel.makeKeyAndOrderFront(nil)
            NSLog("Panel ordered front, isVisible now: \(panel.isVisible)")

            NSAnimationContext.runAnimationGroup({ context in
                context.duration = 0.35
                context.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
                panel.animator().setFrameOrigin(NSPoint(x: openX, y: y))
            }, completionHandler: {
                NSLog("Panel open animation complete, final frame: \(panel.frame)")
            })
        }
    }

    func createMenu() -> NSMenu {
        let menu = NSMenu()

        let serverItem = NSMenuItem(
            title: serverRunning ? "Stop Server" : "Start Server",
            action: #selector(toggleServer),
            keyEquivalent: ""
        )
        serverItem.target = self
        menu.addItem(serverItem)

        let funnelItem = NSMenuItem(
            title: funnelRunning ? "Stop Funnel" : "Start Funnel",
            action: #selector(toggleFunnel),
            keyEquivalent: ""
        )
        funnelItem.target = self
        menu.addItem(funnelItem)

        menu.addItem(NSMenuItem.separator())

        let statusItemMenu = NSMenuItem(
            title: statusText(),
            action: nil,
            keyEquivalent: ""
        )
        statusItemMenu.isEnabled = false
        menu.addItem(statusItemMenu)

        menu.addItem(NSMenuItem.separator())

        let sidePanelItem = NSMenuItem(
            title: "Toggle Sidebar",
            action: #selector(toggleSidePanel),
            keyEquivalent: ""
        )
        sidePanelItem.target = self
        menu.addItem(sidePanelItem)

        let openLocalItem = NSMenuItem(
            title: "Open http://127.0.0.1:9105",
            action: #selector(openLocal),
            keyEquivalent: ""
        )
        openLocalItem.target = self
        menu.addItem(openLocalItem)

        let openPublicItem = NSMenuItem(
            title: "Open Public URL",
            action: #selector(openPublic),
            keyEquivalent: ""
        )
        openPublicItem.target = self
        menu.addItem(openPublicItem)

        menu.addItem(NSMenuItem.separator())

        let connectItem = NSMenuItem(
            title: "Connect MCP",
            action: #selector(connectMCP),
            keyEquivalent: ""
        )
        connectItem.target = self
        menu.addItem(connectItem)

        menu.addItem(NSMenuItem.separator())

        let cleanCaptureItem = NSMenuItem(
            title: "Clean Capture Mode",
            action: #selector(toggleCleanCaptureMode),
            keyEquivalent: ""
        )
        cleanCaptureItem.target = self
        menu.addItem(cleanCaptureItem)

        menu.addItem(NSMenuItem.separator())

        let quitItem = NSMenuItem(
            title: "Quit",
            action: #selector(quit),
            keyEquivalent: "q"
        )
        quitItem.target = self
        menu.addItem(quitItem)

        return menu
    }

    func statusText() -> String {
        var parts: [String] = []
        if serverRunning { parts.append("Server: ON") }
        if funnelRunning { parts.append("Funnel: ON") }
        return parts.isEmpty ? "Status: Idle" : parts.joined(separator: " | ")
    }

    func setCleanCaptureMode(_ enabled: Bool) {
        guard let panel = sidePanel else { return }
        if enabled {
            panel.appearance = nil
            panel.backgroundColor = NSColor.black
            NSLog("Clean capture mode ON")
        } else {
            panel.appearance = NSAppearance(named: .darkAqua)
            panel.backgroundColor = NSColor.clear
            NSLog("Clean capture mode OFF")
        }
    }

    @objc func toggleCleanCaptureMode() {
        guard let panel = sidePanel else { return }
        setCleanCaptureMode(panel.appearance != nil)
    }

    @objc func toggleServer() {
        if serverRunning {
            serverTask?.terminate()
            serverTask = nil
            serverRunning = false
        } else {
            let bunPath = ProcessInfo.processInfo.environment["TRIOS_BUN_PATH"] ?? "/opt/homebrew/bin/bun"
            guard FileManager.default.fileExists(atPath: bunPath) else {
                showAlert("bun not found at \(bunPath). Set TRIOS_BUN_PATH or install with: brew install bun")
                return
            }
            let task = Process()
            task.executableURL = URL(fileURLWithPath: bunPath)
            task.arguments = ["run", "start:server"]
            task.currentDirectoryURL = URL(fileURLWithPath: "/Users/playra/BrowserOS-full/packages/browseros-agent")
            task.environment = [
                "BROWSEROS_CDP_PORT": "9106",
                "BROWSEROS_SERVER_PORT": "9105",
                "PATH": "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
            ]
            do {
                try task.run()
                serverTask = task
                serverRunning = true
                task.terminationHandler = { [weak self] _ in
                    DispatchQueue.main.async {
                        self?.serverRunning = false
                    }
                }
            } catch {
                showAlert("Failed to start server: \(error.localizedDescription)")
            }
        }
    }

    @objc func toggleFunnel() {
        let tailscalePath = ProcessInfo.processInfo.environment["TRIOS_TAILSCALE_PATH"] ?? "/opt/homebrew/bin/tailscale"
        if funnelRunning {
            funnelTask?.terminate()
            funnelTask = nil
            funnelRunning = false
            let offTask = Process()
            offTask.executableURL = URL(fileURLWithPath: "/usr/bin/sudo")
            offTask.arguments = [tailscalePath, "serve", "--https=443", "off"]
            try? offTask.run()
        } else {
            let task = Process()
            task.executableURL = URL(fileURLWithPath: "/usr/bin/sudo")
            task.arguments = [tailscalePath, "serve", "--https=443", "http://127.0.0.1:9105"]
            do {
                try task.run()
                funnelTask = task
                funnelRunning = true
                task.terminationHandler = { [weak self] _ in
                    DispatchQueue.main.async {
                        self?.funnelRunning = false
                    }
                }
            } catch {
                showAlert("Failed to start funnel: \(error.localizedDescription)")
            }
        }
    }

    @objc func openLocal() {
        guard let url = URL(string: "http://127.0.0.1:9105") else { return }
        NSWorkspace.shared.open(url)
    }

    @objc func openPublic() {
        guard let url = URL(string: "https://playras-macbook-pro-1.tail01804b.ts.net") else { return }
        NSWorkspace.shared.open(url)
    }

    @objc func connectMCP() {
        guard let url = URL(string: "browseros://settings") else { return }
        NSWorkspace.shared.open(url)
        let alert = NSAlert()
        alert.messageText = "Connect TRIOS to Local Server"
        alert.informativeText = """
1. In TRIOS Settings, find "Add Custom App"
2. Name: TRIOS Local Filesystem
3. URL: http://127.0.0.1:9105/mcp
4. Click Save

Your filesystem tools will be available!
"""
        alert.alertStyle = .informational
        alert.runModal()
    }

    @objc func quit() {
        Task {
            await chatViewModel?.unregisterA2A()
            await MainActor.run {
                serverTask?.terminate()
                funnelTask?.terminate()
                NSApplication.shared.terminate(nil)
            }
        }
    }

    func showAlert(_ message: String) {
        let alert = NSAlert()
        alert.messageText = message
        alert.alertStyle = .warning
        alert.runModal()
    }
}

// MARK: - Composition Root (Dependency Injection)

struct CompositionRoot {
    func makeChatViewModel() -> ChatViewModel {
        let transport = SSETransport()
        let healthCheck = HealthCheckTransport()
        let parser = UIMessageStreamParser()
        let persister = ConversationPersister()
        let stateMachine = ConversationStateMachine()

        let serverURL = URL(string: "http://127.0.0.1:9105")!
        let agentCard = AgentCard(
            id: AgentId("trios-agent"),
            name: "TRIOS AGENT",
            description: "Commanding General of BrowserOS Agents. Native macOS A2A participant with browser control and chat capabilities.",
            capabilities: [.browserControl, .chat, .orchestrator],
            version: "1.0.0",
            endpoint: URL(string: "http://127.0.0.1:9105/a2a")
        )
        let a2aClient = A2ARegistryClient(serverURL: serverURL, agentCard: agentCard)

        return ChatViewModel(
            transport: transport,
            healthCheck: healthCheck,
            parser: parser,
            persister: persister,
            stateMachine: stateMachine,
            a2aClient: a2aClient
        )
    }
}

let delegate = AppDelegate()
NSApplication.shared.delegate = delegate
NSApplication.shared.run()
