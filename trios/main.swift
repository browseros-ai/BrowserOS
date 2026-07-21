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
        let targetScreen = screen ?? detectScreenForMouse() ?? NSScreen.main ?? NSScreen.screens.first ?? NSScreen()
        guard targetScreen.frame.width > 0 else { return }
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

// MARK: - KeyWindow

class KeyWindow: NSWindow {
    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { true }
}

// MARK: - AppDelegate

class AppDelegate: NSObject, NSApplicationDelegate {
    var statusItem: NSStatusItem?
    let windowManager = WindowManager()
    let serverManager = ServerManager()
    let screenManager = TriosScreenManager.shared
    let compositionRoot = CompositionRoot()
    lazy var menuBuilder = MenuBuilder(delegate: self, screenManager: screenManager, serverManager: serverManager)

    var chatViewModel: ChatViewModel?
    var sessionGuard: SessionGuard?
    var cladeGuard: CladeGuard?
    var accessibilityGranted = false
    var accessibilityPromptShown = false
    var windowStates: [(AXUIElement, CGRect)] = []
    let sidebarWidth: CGFloat = 400
    var currentSidebarWidth: CGFloat = 400

    func applicationDidFinishLaunching(_ notification: Notification) {
        // SAFETY: Prevent recursive self-launch — enforce single instance
        guard RecursionGuard.shared.ensureSingleInstance() else {
            NSLog("applicationDidFinishLaunching: another trios instance is already running — terminating")
            NSApplication.shared.terminate(nil)
            return
        }
        NSLog("applicationDidFinishLaunching called")
        NSApplication.shared.setActivationPolicy(.regular)
        NSApplication.shared.activate(ignoringOtherApps: true)

        setupStatusItem()
        // CRITICAL: setupSidePanel MUST run synchronously before any UI interaction.
        // Previously it was in Task { @MainActor in } which meant panel was nil
        // when the user clicked the status bar icon before the task completed.
        setupSidePanel()
        accessibilityGranted = AXIsProcessTrusted()
        setupGlobalHotkey()

        Task { @MainActor in
            if let vm = chatViewModel {
                let guard_ = compositionRoot.makeSessionGuard(for: vm)
                sessionGuard = guard_
                guard_.startMonitoring()
            }
            // CladeGuard monitors Sovereign + Canary health and auto-rollback
            let cg = compositionRoot.makeCladeGuard()
            cladeGuard = cg
            cg.startMonitoring()
            await chatViewModel?.registerA2A()
        }
    }

    private func setupStatusItem() {
        // Guard against duplicate status items if this method is called more than once
        if let existing = statusItem, existing.button != nil {
            NSLog("setupStatusItem: statusItem already exists, skipping creation")
            return
        }

        NSLog("setupStatusItem starting")
        var logoImage: NSImage?
        var isOriginalLogo = false
        if let logoURL = Bundle.main.url(forResource: "logo", withExtension: "png"),
           let image = NSImage(contentsOf: logoURL) {
            logoImage = image
            isOriginalLogo = true
        } else {
            let fallbackPaths = [
                ProjectPaths.logoPNG,
                "\(ProjectPaths.appBundle)/Contents/Resources/logo.png"
            ]
            for path in fallbackPaths {
                if FileManager.default.fileExists(atPath: path),
                   let image = NSImage(contentsOfFile: path) {
                    logoImage = image
                    isOriginalLogo = true
                    break
                }
            }
        }

        // Fallback: generate a solid-color circle icon so the user ALWAYS has a visible status icon
        if logoImage == nil {
            logoImage = generateFallbackIcon()
            NSLog("setupStatusItem: generated fallback circle icon")
        }

        if let image = logoImage {
            statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
            // CRITICAL: isTemplate must be false for the colored fallback icon.
            // isTemplate = true tells macOS to recolor the image to match the menubar
            // (black/white), which makes a colored circle invisible. Only use template
            // mode for the original monochrome logo file.
            image.isTemplate = isOriginalLogo
            image.size = NSSize(width: 22, height: 22)
            statusItem?.button?.image = image
            statusItem?.button?.imagePosition = .imageOnly
            statusItem?.button?.title = ""
            NSLog("setupStatusItem: image set (template=\(isOriginalLogo)), size=\(image.size)")
        } else {
            statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
            statusItem?.button?.title = "T"
            statusItem?.button?.font = NSFont.systemFont(ofSize: 14, weight: .bold)
            NSLog("setupStatusItem: no image, using text fallback 'T'")
        }
        statusItem?.button?.toolTip = "TRIOS AGENT"
        statusItem?.button?.target = self
        statusItem?.button?.action = #selector(statusBarButtonClicked(_:))
        statusItem?.button?.sendAction(on: [.leftMouseUp, .rightMouseUp])
        statusItem?.menu = nil
        if let button = statusItem?.button {
            NSLog("setupStatusItem done — statusItem created, button frame=\(button.frame), image=\(String(describing: button.image))")
        } else {
            NSLog("setupStatusItem ERROR: statusItem.button is nil!")
        }
    }

    /// Generates a solid colored circle NSImage so the status bar icon is always visible
    private func generateFallbackIcon() -> NSImage {
        let size = NSSize(width: 22, height: 22)
        let image = NSImage(size: size)
        image.lockFocus()
        let rect = NSRect(origin: .zero, size: size)
        let path = NSBezierPath(ovalIn: rect)
        NSColor(red: 0.45, green: 0.55, blue: 1.0, alpha: 1.0).setFill()
        path.fill()
        image.unlockFocus()
        return image
    }

    @MainActor
    func setupSidePanel() {
        NSLog("setupSidePanel starting")
        let viewModel = compositionRoot.makeChatViewModel()
        self.chatViewModel = viewModel
        NSLog("ChatViewModel created")
        let tabView = TriosTabView(viewModel: viewModel)
        NSLog("TriosTabView created")
        let panel = windowManager.setupPanel(contentView: AnyView(tabView))
        NSLog("Panel created: \(panel)")
    }

    // MARK: - Window Shifting

    func getWindowFrame(_ window: AXUIElement) -> CGRect? {
        var positionValue: CFTypeRef?
        guard AXUIElementCopyAttributeValue(window, kAXPositionAttribute as CFString, &positionValue) == .success else {
            return nil
        }
        guard CFGetTypeID(positionValue) == AXValueGetTypeID() else { return nil }
        var position = CGPoint.zero
        // CFGetTypeID check above guarantees AXValue type. `as!` is the required
        // idiomatic form for CoreFoundation types here — `as?` is rejected by
        // the compiler ("conditional downcast ... will always succeed").
        guard AXValueGetValue(positionValue as! AXValue, .cgPoint, &position) else { return nil }

        var sizeValue: CFTypeRef?
        guard AXUIElementCopyAttributeValue(window, kAXSizeAttribute as CFString, &sizeValue) == .success else {
            return nil
        }
        guard CFGetTypeID(sizeValue) == AXValueGetTypeID() else { return nil }
        var size = CGSize.zero
        // CFGetTypeID-checked; `as!` required for CoreFoundation cast (see above).
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
            NSLog("statusBarButtonClicked: no current event, toggling")
            toggleSidePanel()
            return
        }
        NSLog("statusBarButtonClicked: event.type = \(event.type)")
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
        // CRITICAL: Lazy creation — if panel hasn't been built yet, build it now.
        // This prevents the "click does nothing" bug when setupSidePanel hasn't completed.
        Task { @MainActor in
            if windowManager.panel == nil {
                NSLog("toggleSidePanel: panel is nil — creating lazily")
                setupSidePanel()
            }
            performToggle()
        }
    }

    @MainActor
    private func performToggle() {
        guard let panel = windowManager.panel else {
            NSLog("toggleSidePanel: panel is STILL nil after lazy creation — critical error")
            return
        }
        NSLog("toggleSidePanel: panel isVisible=\(panel.isVisible), frame=\(panel.frame)")
        if panel.isVisible {
            NSLog("toggleSidePanel: closing panel")
            windowManager.close { [weak self] in
                self?.restoreWindows()
            }
        } else {
            NSLog("toggleSidePanel: opening panel")
            accessibilityGranted = AXIsProcessTrusted()
            NSLog("toggleSidePanel: accessibilityGranted=\(accessibilityGranted)")
            if accessibilityGranted {
                shiftWindows()
            } else if !accessibilityPromptShown {
                accessibilityPromptShown = true
                showAlert("Please grant Trios Accessibility access in:\nSystem Settings → Privacy & Security → Accessibility")
            }
            NSLog("toggleSidePanel: calling windowManager.open()")
            windowManager.open()
            NSLog("toggleSidePanel: windowManager.open() returned")
        }
    }

    func createMenu() -> NSMenu {
        menuBuilder.buildMenu()
    }

    func statusText() -> String {
        menuBuilder.statusText()
    }

    func setCleanCaptureMode(_ enabled: Bool) {
        guard let panel = windowManager.panel else { return }
        screenManager.setCleanCaptureMode(enabled, for: panel)
    }

    @objc func toggleCleanCaptureMode() {
        guard let panel = windowManager.panel else { return }
        setCleanCaptureMode(panel.appearance != nil)
    }

    @objc func toggleServer() {
        serverManager.toggleServer()
    }

    @objc func toggleFunnel() {
        serverManager.toggleFunnel()
    }

    @objc func openLocal() {
        guard let url = URL(string: ProjectPaths.mcpBaseURL) else { return }
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
        RecursionGuard.shared.cleanup()
        Task {
            await chatViewModel?.unregisterA2A()
            await MainActor.run {
                serverManager.terminateAll()
                NSApplication.shared.terminate(nil)
            }
        }
    }

    // MARK: - URL / Reopen Safety Guards

    func application(_ application: NSApplication, open urls: [URL]) {
        for url in urls {
            let urlString = url.absoluteString.lowercased()
            // Block any URL that could trigger a recursive self-launch
            if urlString.contains("trios") || urlString.contains("browseros://settings") {
                NSLog("[AppDelegate] Blocked recursive URL open: \(url)")
                continue
            }
            NSWorkspace.shared.open(url)
        }
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        // If panel is already visible, just focus it instead of creating duplicate state
        if let panel = windowManager.panel {
            panel.makeKeyAndOrderFront(nil)
            NSApplication.shared.activate(ignoringOtherApps: true)
            return false
        }
        return true
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
        if let panel = windowManager.panel {
            screenManager.applyMode(to: panel)
        }
    }

    @objc func moveToScreen(_ sender: NSMenuItem) {
        let screens = NSScreen.screens
        guard sender.tag < screens.count else { return }
        if let panel = windowManager.panel {
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
    @MainActor
    func makeChatViewModel() -> ChatViewModel {
        NSLog("CompositionRoot: creating ChatViewModel...")
        let transport = SSETransport()
        NSLog("CompositionRoot: SSETransport created")
        let healthCheck = HealthCheckTransport()
        let parser = UIMessageStreamParser()
        let persister = ConversationPersister()
        let stateMachine = ConversationStateMachine()

        let serverURL = URL(string: ProjectPaths.mcpBaseURL) ?? URL(fileURLWithPath: "/dev/null")
        let agentCard = AgentCard(
            id: AgentId("trios-agent"),
            name: "TRIOS AGENT",
            description: "Commanding General of BrowserOS Agents. Native macOS A2A participant with browser control and chat capabilities.",
            capabilities: [.browserControl, .chat, .orchestrator],
            version: "1.0.0",
            endpoint: URL(string: "\(ProjectPaths.mcpBaseURL)/a2a")
        )
        let a2aClient = A2ARegistryClient(serverURL: serverURL, agentCard: agentCard)
        NSLog("CompositionRoot: A2ARegistryClient created")

        let vm = ChatViewModel(
            transport: transport,
            healthCheck: healthCheck,
            parser: parser,
            persister: persister,
            stateMachine: stateMachine,
            a2aClient: a2aClient
        )
        NSLog("CompositionRoot: ChatViewModel created")
        return vm
    }

    @MainActor
    func makeSessionGuard(for viewModel: ChatViewModel) -> SessionGuard {
        let healthCheck = HealthCheckTransport()
        return SessionGuard(healthCheck: healthCheck, a2aClient: viewModel.a2aClient)
    }

    @MainActor
    func makeCladeGuard() -> CladeGuard {
        return CladeGuard()
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

// SAFETY: Enforce single instance before any UI is created or the run loop starts.
// This catches launches that would otherwise race through AppDelegate initialization.
guard RecursionGuard.shared.ensureSingleInstance() else {
    NSLog("[main] Another trios instance is already running — exiting")
    exit(0)
}

let delegate = AppDelegate()
NSApplication.shared.delegate = delegate
NSApplication.shared.run()
