import Cocoa
import Foundation

/// Manages the BrowserOS MCP server (bun) and Tailscale funnel processes.
final class ServerManager {
    private var serverTask: Process?
    private var funnelTask: Process?
    private(set) var serverRunning = false
    private(set) var funnelRunning = false

    var onStatusChange: (() -> Void)?

    // MARK: - Server

    func toggleServer() {
        if serverRunning {
            serverTask?.terminate()
            serverTask = nil
            serverRunning = false
            onStatusChange?()
        } else {
            let bunPath = ProcessInfo.processInfo.environment["TRIOS_BUN_PATH"] ?? "/opt/homebrew/bin/bun"
            guard FileManager.default.fileExists(atPath: bunPath) else {
                showAlert("bun not found at \(bunPath). Set TRIOS_BUN_PATH or install with: brew install bun")
                return
            }
            let task = Process()
            task.executableURL = URL(fileURLWithPath: bunPath)
            task.arguments = ["run", "start:server"]
            task.currentDirectoryURL = URL(fileURLWithPath: ProjectPaths.browserOSAgentRoot)
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
                        self?.onStatusChange?()
                    }
                }
                onStatusChange?()
            } catch {
                showAlert("Failed to start server: \(error.localizedDescription)")
            }
        }
    }

    // MARK: - Funnel

    func toggleFunnel() {
        let tailscalePath = ProcessInfo.processInfo.environment["TRIOS_TAILSCALE_PATH"] ?? "/opt/homebrew/bin/tailscale"
        if funnelRunning {
            funnelTask?.terminate()
            funnelTask = nil
            funnelRunning = false
            let offTask = Process()
            offTask.executableURL = URL(fileURLWithPath: "/usr/bin/sudo")
            offTask.arguments = [tailscalePath, "serve", "--https=443", "off"]
            try? offTask.run()
            onStatusChange?()
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
                        self?.onStatusChange?()
                    }
                }
                onStatusChange?()
            } catch {
                showAlert("Failed to start funnel: \(error.localizedDescription)")
            }
        }
    }

    // MARK: - Cleanup

    func terminateAll() {
        serverTask?.terminate()
        funnelTask?.terminate()
        serverTask = nil
        funnelTask = nil
        serverRunning = false
        funnelRunning = false
    }

    // MARK: - Helpers

    private func showAlert(_ message: String) {
        let alert = NSAlert()
        alert.messageText = message
        alert.alertStyle = .warning
        alert.runModal()
    }
}
