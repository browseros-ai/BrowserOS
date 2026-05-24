import Cocoa
import Foundation
import SwiftUI
import ApplicationServices

// MARK: - Panel Mode Enum

enum TriosPanelMode: String, CaseIterable {
    case glassmorphismSidebar = "Glassmorphism Sidebar"
    case floatingSidebar = "Floating Sidebar"
    case hudMinimal = "HUD Minimal"
    case borderlessOverlay = "Borderless Overlay"
    case stationaryWidget = "Stationary Widget"

    var styleMask: NSWindow.StyleMask {
        switch self {
        case .glassmorphismSidebar:
            return [.fullSizeContentView]
        case .floatingSidebar:
            return [.titled, .closable, .utilityWindow]
        case .hudMinimal:
            return [.hudWindow, .nonactivatingPanel]
        case .borderlessOverlay:
            return [.borderless, .fullSizeContentView]
        case .stationaryWidget:
            return [.titled, .closable, .utilityWindow]
        }
    }

    var collectionBehavior: NSWindow.CollectionBehavior {
        switch self {
        case .glassmorphismSidebar:
            return [.canJoinAllSpaces, .fullScreenAuxiliary, .transient, .ignoresCycle]
        case .floatingSidebar:
            return [.transient, .canJoinAllSpaces, .fullScreenAuxiliary]
        case .hudMinimal:
            return [.transient, .canJoinAllSpaces, .fullScreenAuxiliary, .ignoresCycle]
        case .borderlessOverlay:
            return [.transient, .canJoinAllSpaces, .ignoresCycle]
        case .stationaryWidget:
            return [.stationary, .canJoinAllSpaces, .ignoresCycle]
        }
    }

    var level: NSWindow.Level {
        switch self {
        case .glassmorphismSidebar: return .mainMenu
        case .floatingSidebar: return .normal
        case .hudMinimal: return .floating
        case .borderlessOverlay: return .popUpMenu
        case .stationaryWidget: return .normal
        }
    }

    var isFloatingPanel: Bool {
        switch self {
        case .glassmorphismSidebar, .hudMinimal: return true
        default: return false
        }
    }

    var isOpaque: Bool {
        switch self {
        case .borderlessOverlay, .glassmorphismSidebar: return false
        default: return true
        }
    }

    var backgroundColor: NSColor? {
        switch self {
        case .borderlessOverlay, .glassmorphismSidebar: return .clear
        default: return nil
        }
    }
}

// MARK: - Screen Management

class TriosScreenManager {
    static let shared = TriosScreenManager()

    var currentScreen: NSScreen?
    var panelMode: TriosPanelMode = .glassmorphismSidebar

    func detectScreenForMouse() -> NSScreen? {
        let mouseLocation = NSEvent.mouseLocation
        return NSScreen.screens.first { screen in
            screen.frame.contains(mouseLocation)
        } ?? NSScreen.main
    }

    func positionPanel(_ panel: NSWindow, on screen: NSScreen? = nil, width: CGFloat = 400) {
        let targetScreen = screen ?? detectScreenForMouse() ?? NSScreen.main ?? NSScreen.screens.first!
        let frame = targetScreen.visibleFrame

        let panelHeight = frame.height
        let x = frame.maxX - width
        let y = frame.minY

        panel.setFrame(NSRect(x: x, y: y, width: width, height: panelHeight), display: true, animate: false)
        currentScreen = targetScreen
    }

    func applyMode(to panel: NSWindow) {
        panel.styleMask = panelMode.styleMask
        panel.collectionBehavior = panelMode.collectionBehavior
        panel.level = panelMode.level
        panel.isOpaque = panelMode.isOpaque
        if let color = panelMode.backgroundColor {
            panel.backgroundColor = color
        } else {
            panel.backgroundColor = nil
        }
    }

    func setCleanCaptureMode(_ enabled: Bool, for panel: NSWindow) {
        if enabled {
            panel.appearance = nil
            panel.backgroundColor = .black
            panel.isOpaque = true
        } else {
            panel.appearance = NSAppearance(named: .darkAqua)
            applyMode(to: panel)
        }
    }

    func cycleToNextMode() {
        let all = TriosPanelMode.allCases
        let nextIndex = (all.firstIndex(of: panelMode)! + 1) % all.count
        panelMode = all[nextIndex]
    }
}

// MARK: - AppDelegate

class KeyWindow: NSWindow {
    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { true }
}

class AppDelegate: NSObject, NSApplicationDelegate {
    var statusItem: NSStatusItem?
    var sidePanel: NSWindow?
    var hostingController: NSHostingController<TriosTabView>?
    var serverTask: Process?
    var funnelTask: Process?
    var serverRunning = false
    var funnelRunning = false
    var windowStates: [(AXUIElement, CGRect)] = []
    let sidebarWidth: CGFloat = 400
    var currentSidebarWidth: CGFloat = 400
    let compositionRoot = CompositionRoot()
    var accessibilityGranted = false
    var accessibilityPromptShown = false
    var chatViewModel: ChatViewModel?
    var screenManager = TriosScreenManager.shared

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

        statusItem?.button?.target = self
        statusItem?.button?.action = #selector(statusBarButtonClicked(_:))
        statusItem?.button?.sendAction(on: [.leftMouseUp, .rightMouseUp])
        statusItem?.menu = nil

        setupSidePanel()
        accessibilityGranted = AXIsProcessTrusted()
        setupGlobalHotkey()

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

        let panel = KeyWindow(
            contentRect: NSRect(x: x, y: y, width: sidebarWidth, height: panelHeight),
            styleMask: [.borderless, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )

        panel.level = .floating
        panel.hidesOnDeactivate = false
        panel.isMovableByWindowBackground = true
        panel.backgroundColor = NSColor.clear
        panel.isOpaque = false
        panel.hasShadow = true
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .transient, .ignoresCycle]
        panel.animationBehavior = .none
        panel.appearance = NSAppearance(named: .darkAqua)
        panel.isReleasedWhenClosed = false

        let viewModel = compositionRoot.makeChatViewModel()
        self.chatViewModel = viewModel
        let tabView = TriosTabView(viewModel: viewModel) { [weak self] width in
            self?.resizePanel(to: width)
        }
        let hc = NSHostingController(rootView: tabView)
        hc.view.frame = panel.contentView!.bounds
        hc.view.autoresizingMask = [.width, .height]
        panel.contentView = hc.view
        NSLog("SwiftUI hosting view set as contentView, frame: \(hc.view.frame)")

        self.hostingController = hc
        self.sidePanel = panel
        screenManager.currentScreen = screen
    }

    func resizePanel(to width: CGFloat) {
        guard let panel = sidePanel else { return }
        guard width != currentSidebarWidth else { return }
        currentSidebarWidth = width

        let screen = screenManager.currentScreen ?? NSScreen.main
        let screenFrame = screen?.visibleFrame ?? NSScreen.main?.visibleFrame ?? CGRect(x: 0, y: 0, width: 1512, height: 982)
        let panelHeight = screenFrame.height
        let x = screenFrame.maxX - width
        let y = screenFrame.minY

        NSAnimationContext.runAnimationGroup({ context in
            context.duration = 0.25
            context.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
            panel.animator().setFrame(NSRect(x: x, y: y, width: width, height: panelHeight), display: true)
        }, completionHandler: {
            NSLog("Panel resized to width \(width)")
        })
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
        let cutoffX = screenFrame.maxX - currentSidebarWidth
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
            let screen = screenManager.currentScreen ?? NSScreen.main
            let screenFrame = screen?.visibleFrame ?? NSScreen.main?.visibleFrame ?? CGRect(x: 0, y: 0, width: 1512, height: 982)
            let offscreenX = screenFrame.maxX
            let closeY = panel.frame.origin.y
            let closeHeight = panel.frame.height
            NSAnimationContext.runAnimationGroup({ context in
                context.duration = 0.35
                context.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
                panel.animator().setFrame(NSRect(x: offscreenX, y: closeY, width: self.currentSidebarWidth, height: closeHeight), display: true)
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
            guard let screen = screenManager.detectScreenForMouse() ?? NSScreen.main else {
                return
            }
            NSLog("Got screen: \(screen.frame)")
            let screenFrame = screen.visibleFrame
            let panelHeight = screenFrame.height
            let openX = screenFrame.maxX - self.currentSidebarWidth
            let offscreenX = screenFrame.maxX
            let y = screenFrame.minY

            panel.setFrame(NSRect(x: offscreenX, y: y, width: self.currentSidebarWidth, height: panelHeight), display: false)
            NSLog("Panel positioned off-screen at x=\(offscreenX), about to order front")

            panel.makeKeyAndOrderFront(nil)
            NSApplication.shared.activate(ignoringOtherApps: true)
            NSLog("Panel is key window: \(panel.isKeyWindow), ordered front")

            NSAnimationContext.runAnimationGroup({ context in
                NSLog("Animation group starting, target x=\(openX)")
                context.duration = 0.35
                context.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
                panel.animator().setFrame(NSRect(x: openX, y: y, width: self.currentSidebarWidth, height: panelHeight), display: true)
            }, completionHandler: {
                NSLog("Panel open animation complete, final frame: \(panel.frame), isVisible: \(panel.isVisible)")
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

        // Panel Mode submenu
        let modeMenu = NSMenu(title: "Panel Mode")
        for (index, mode) in TriosPanelMode.allCases.enumerated() {
            let item = NSMenuItem(title: mode.rawValue, action: #selector(setPanelMode(_:)), keyEquivalent: "")
            item.tag = index
            item.target = self
            item.state = (mode == screenManager.panelMode) ? .on : .off
            modeMenu.addItem(item)
        }
        let modeItem = NSMenuItem(title: "Panel Mode", action: nil, keyEquivalent: "")
        modeItem.submenu = modeMenu
        menu.addItem(modeItem)

        // Move to Screen submenu
        let screenMenu = NSMenu(title: "Move to Screen")
        for (index, screen) in NSScreen.screens.enumerated() {
            let name = screen.displayName ?? "Screen \(index + 1)"
            let item = NSMenuItem(title: name, action: #selector(moveToScreen(_:)), keyEquivalent: "")
            item.tag = index
            item.target = self
            screenMenu.addItem(item)
        }
        let screenItem = NSMenuItem(title: "Move to Screen", action: nil, keyEquivalent: "")
        screenItem.submenu = screenMenu
        menu.addItem(screenItem)

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
        screenManager.setCleanCaptureMode(enabled, for: panel)
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

    // MARK: - Panel Mode Actions

    @objc func setPanelMode(_ sender: NSMenuItem) {
        let modes = TriosPanelMode.allCases
        guard sender.tag < modes.count else { return }
        screenManager.panelMode = modes[sender.tag]
        if let panel = sidePanel {
            screenManager.applyMode(to: panel)
        }
    }

    @objc func moveToScreen(_ sender: NSMenuItem) {
        let screens = NSScreen.screens
        guard sender.tag < screens.count else { return }
        if let panel = sidePanel {
            screenManager.positionPanel(panel, on: screens[sender.tag], width: sidebarWidth)
        }
    }

    // MARK: - Global Hotkey

    func setupGlobalHotkey() {
        NSEvent.addGlobalMonitorForEvents(matching: .keyDown) { [weak self] event in
            guard event.modifierFlags.contains(.command),
                  event.modifierFlags.contains(.shift),
                  event.keyCode == 17 else { return }
            DispatchQueue.main.async {
                self?.toggleSidePanel()
            }
        }
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

// MARK: - Screen Extension

extension NSScreen {
    var displayName: String? {
        guard let screenNumber = deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? CGDirectDisplayID else {
            return nil
        }
        return "Display \(screenNumber)"
    }
}

let delegate = AppDelegate()
NSApplication.shared.delegate = delegate
NSApplication.shared.run()
