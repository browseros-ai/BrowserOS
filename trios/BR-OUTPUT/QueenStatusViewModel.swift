import Foundation
import SwiftUI

enum ComponentStatus: String {
    case healthy = "ok"
    case warning = "warn"
    case down = "down"
    case unknown = "unknown"
}

struct StatusComponent: Identifiable {
    let id = UUID()
    let name: String
    let icon: String
    let status: ComponentStatus
    let detail: String
    let actionLabel: String?
}

struct SkillRun: Identifiable {
    let id = UUID()
    let name: String
    let lastRun: Date?
    let success: Bool?
    let isRunning: Bool
}

@MainActor
final class QueenStatusViewModel: ObservableObject {
    @Published var components: [StatusComponent] = []
    @Published var skills: [SkillRun] = []
    @Published var lastLogLines: [String] = []
    @Published var isRunningAction: Bool = false
    @Published var overallStatus: ComponentStatus = .unknown

    private let projectRoot = ProjectPaths.root
    private let statePath = ProjectPaths.trinityState
    private let logPath = ProjectPaths.trinityLog

    private var refreshTimer: Timer?
    private var logTimer: Timer?

    init() {
        refreshAll()
        startTimers()
    }

    deinit {
        refreshTimer?.invalidate()
        logTimer?.invalidate()
    }

    private func startTimers() {
        refreshTimer = Timer.scheduledTimer(withTimeInterval: 10, repeats: true) { _ in
            Task { @MainActor in
                self.refreshAll()
            }
        }
        logTimer = Timer.scheduledTimer(withTimeInterval: 5, repeats: true) { _ in
            Task { @MainActor in
                self.loadLogTail()
            }
        }
    }

    func refreshAll() {
        checkTrios()
        checkMCP()
        checkAgent()
        checkCron()
        checkA2A()
        checkFunnel()
        checkGit()
        checkBuild()
        loadSkills()
        loadLogTail()
        computeOverallStatus()
    }

    // MARK: - Component Checks

    private func checkTrios() {
        let running = shell("pgrep -x trios_app >/dev/null 2>&1 && echo 1 || echo 0").trimmingCharacters(in: .whitespaces) == "1"
        updateComponent(name: "TRIOS", icon: "macwindow", status: running ? .healthy : .down, detail: running ? "Running" : "Stopped", action: running ? nil : "Start")
    }

    private func checkMCP() {
        let healthy = shell("curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:9105/health 2>/dev/null || echo 000").trimmingCharacters(in: .whitespaces) == "200"
        updateComponent(name: "MCP", icon: "server.rack", status: healthy ? .healthy : .down, detail: healthy ? "Online" : "Offline", action: healthy ? "Restart" : "Start")
    }

    private func checkAgent() {
        let healthy = shell("curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:9200/health 2>/dev/null || echo 000").trimmingCharacters(in: .whitespaces) == "200"
        updateComponent(name: "Agent", icon: "cpu", status: healthy ? .healthy : .down, detail: healthy ? "Online" : "Offline", action: healthy ? "Restart" : "Start")
    }

    private func checkCron() {
        let fm = FileManager.default
        guard fm.fileExists(atPath: statePath) else {
            updateComponent(name: "Cron", icon: "clock.arrow.circlepath", status: .unknown, detail: "No state", action: "Run")
            return
        }
        do {
            let data = try Data(contentsOf: URL(fileURLWithPath: statePath))
            if let json = try JSONSerialization.jsonObject(with: data) as? [String: Any],
               let ts = json["ts"] as? TimeInterval {
                let lastWake = Date(timeIntervalSince1970: ts)
                let minutes = Int(Date().timeIntervalSince(lastWake) / 60)
                let health = json["health"] as? String ?? "?"
                let build = json["build"] as? String ?? "?"
                let dirty = json["dirty"] as? Int ?? 0

                let status: ComponentStatus
                let detail: String
                if minutes < 20 {
                    status = health == "ok" ? .healthy : .warning
                    detail = "\(minutes)m ago · build \(build) · dirty \(dirty)"
                } else {
                    status = .warning
                    detail = "\(minutes)m ago (stale)"
                }
                updateComponent(name: "Cron", icon: "clock.arrow.circlepath", status: status, detail: detail, action: "Run")
            } else {
                updateComponent(name: "Cron", icon: "clock.arrow.circlepath", status: .unknown, detail: "Invalid state", action: "Run")
            }
        } catch {
            updateComponent(name: "Cron", icon: "clock.arrow.circlepath", status: .unknown, detail: "Error", action: "Run")
        }
    }

    private func checkA2A() {
        let agents = shell("ls \(ProjectPaths.claude("agents"))/*.md 2>/dev/null | wc -l | tr -d ' '")
        let count = Int(agents.trimmingCharacters(in: .whitespaces)) ?? 0
        let detail = count > 0 ? "\(count) agents" : "No agents"
        updateComponent(name: "A2A", icon: "network", status: count > 0 ? .healthy : .warning, detail: detail, action: nil)
    }

    private func checkFunnel() {
        let running = shell("pgrep -x tailscale >/dev/null 2>&1 && echo 1 || echo 0").trimmingCharacters(in: .whitespaces) == "1"
        updateComponent(name: "Funnel", icon: "globe", status: running ? .healthy : .warning, detail: running ? "Tailscale active" : "Not running", action: nil)
    }

    private func checkGit() {
        let branch = shell("cd \(projectRoot) && git branch --show-current 2>/dev/null || echo '—'")
        let dirty = shell("cd \(projectRoot) && git status --porcelain 2>/dev/null | wc -l | tr -d ' '")
        let dirtyCount = Int(dirty.trimmingCharacters(in: .whitespaces)) ?? 0
        let status: ComponentStatus = dirtyCount > 0 ? .warning : .healthy
        let detail = "\(branch.trimmingCharacters(in: .whitespaces)) · \(dirtyCount) dirty"
        updateComponent(name: "Git", icon: "arrow.triangle.branch", status: status, detail: detail, action: nil)
    }

    private func checkBuild() {
        let result = shell("cd \(projectRoot) && swiftc -typecheck main.swift rings/**/*.swift BR-OUTPUT/*.swift 2>&1 | head -5")
        let ok = result.trimmingCharacters(in: .whitespaces).isEmpty
        updateComponent(name: "Build", icon: "hammer", status: ok ? .healthy : .down, detail: ok ? "OK" : "Errors", action: nil)
    }

    private func updateComponent(name: String, icon: String, status: ComponentStatus, detail: String, action: String?) {
        if let index = components.firstIndex(where: { $0.name == name }) {
            components[index] = StatusComponent(name: name, icon: icon, status: status, detail: detail, actionLabel: action)
        } else {
            components.append(StatusComponent(name: name, icon: icon, status: status, detail: detail, actionLabel: action))
        }
    }

    private func computeOverallStatus() {
        let statuses = components.map { $0.status }
        if statuses.contains(.down) {
            overallStatus = .down
        } else if statuses.contains(.warning) {
            overallStatus = .warning
        } else if statuses.allSatisfy({ $0 == .healthy }) && !components.isEmpty {
            overallStatus = .healthy
        } else {
            overallStatus = .unknown
        }
    }

    // MARK: - Skills

    private func loadSkills() {
        let skillNames = ["/tri", "/doctor", "/god-mode", "/bridge"]
        var result: [SkillRun] = []
        for name in skillNames {
            if let existing = skills.first(where: { $0.name == name }) {
                result.append(existing)
            } else {
                result.append(SkillRun(name: name, lastRun: nil, success: nil, isRunning: false))
            }
        }
        skills = result
    }

    // MARK: - Log

    func loadLogTail() {
        let fm = FileManager.default
        guard fm.fileExists(atPath: logPath) else {
            lastLogLines = ["No cron log found"]
            return
        }
        let output = shell("tail -n 20 \(logPath)")
        lastLogLines = output.split(separator: "\n").map { String($0) }
        if lastLogLines.isEmpty {
            lastLogLines = ["Log empty"]
        }
    }

    // MARK: - Actions

    func startTrios() {
        isRunningAction = true
        runAsync("cd \(projectRoot) && ./trios_app &")
        DispatchQueue.main.asyncAfter(deadline: .now() + 2) {
            self.refreshAll()
            self.isRunningAction = false
        }
    }

    func stopTrios() {
        isRunningAction = true
        runAsync("pkill -9 trios_app || true")
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) {
            self.refreshAll()
            self.isRunningAction = false
        }
    }

    func restartMCP() {
        isRunningAction = true
        runAsync("pkill -f 'bun.*start:server' || true")
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) {
            self.runAsync("cd \(self.projectRoot) && bun run start:server &")
            DispatchQueue.main.asyncAfter(deadline: .now() + 3) {
                self.refreshAll()
                self.isRunningAction = false
            }
        }
    }

    func restartAgent() {
        isRunningAction = true
        runAsync("pkill -f 'bun.*agent' || true")
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) {
            self.runAsync("cd \(self.projectRoot) && bun run start:agent &")
            DispatchQueue.main.asyncAfter(deadline: .now() + 3) {
                self.refreshAll()
                self.isRunningAction = false
            }
        }
    }

    func runCron() {
        isRunningAction = true
        runAsync("cd \(projectRoot) && ./cron-queen.sh || true")
        DispatchQueue.main.asyncAfter(deadline: .now() + 2) {
            self.refreshAll()
            self.isRunningAction = false
        }
    }

    func runSkill(name: String) {
        guard let index = skills.firstIndex(where: { $0.name == name }) else { return }
        skills[index] = SkillRun(name: name, lastRun: skills[index].lastRun, success: skills[index].success, isRunning: true)
        objectWillChange.send()

        runAsync("cd \(projectRoot) && claude \(name)")
        DispatchQueue.main.asyncAfter(deadline: .now() + 5) {
            if let idx = self.skills.firstIndex(where: { $0.name == name }) {
                self.skills[idx] = SkillRun(name: name, lastRun: Date(), success: true, isRunning: false)
                self.objectWillChange.send()
            }
            self.refreshAll()
        }
    }

    func runCommand(_ cmd: String) {
        isRunningAction = true
        runAsync("cd \(projectRoot) && \(cmd)")
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) {
            self.loadLogTail()
            self.isRunningAction = false
        }
    }

    // MARK: - Shell Helpers

    @discardableResult
    private func shell(_ command: String) -> String {
        let task = Process()
        task.executableURL = URL(fileURLWithPath: "/bin/zsh")
        task.arguments = ["-c", command]
        task.currentDirectoryURL = URL(fileURLWithPath: projectRoot)

        let pipe = Pipe()
        task.standardOutput = pipe
        task.standardError = pipe

        do {
            try task.run()
            task.waitUntilExit()
        } catch {
            return ""
        }

        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        return String(data: data, encoding: .utf8) ?? ""
    }

    private func runAsync(_ command: String) {
        let task = Process()
        task.executableURL = URL(fileURLWithPath: "/bin/zsh")
        task.arguments = ["-c", command]
        task.currentDirectoryURL = URL(fileURLWithPath: projectRoot)
        do {
            try task.run()
        } catch {
            NSLog("[QueenStatus] Command failed: \(error)")
        }
    }
}
