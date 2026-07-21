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

/// Agent control model for A2A agent lifecycle management.
struct AgentInfo: Identifiable {
    let id = UUID()
    let name: String
    let status: ComponentStatus
    let pid: Int?
    let uptime: String
    let lastAction: String?
}

struct SelfImprovementStatus {
    let score: Int
    let openIssues: Int
    let lastAuditAgo: String
    let safetyBudget: String
    let lastCritique: String
    let pendingPRs: Int
}

struct AuditRecord {
    let timestamp: Date
    let findings: Int
    let passed: Bool
}

@MainActor
final class QueenStatusViewModel: ObservableObject {
    @Published var components: [StatusComponent] = []
    @Published var skills: [SkillRun] = []
    @Published var agents: [AgentInfo] = []
    @Published var lastLogLines: [String] = []
    @Published var isRunningAction: Bool = false
    @Published var overallStatus: ComponentStatus = .unknown
    @Published var selfImprovement: SelfImprovementStatus? = nil
    @Published var auditHistory: [AuditRecord] = []

    private let projectRoot = ProjectPaths.root
    private let statePath = ProjectPaths.trinityState
    private let logPath = ProjectPaths.trinityLog

    private var refreshTimer: Timer?
    private var logTimer: Timer?

    init() {
        startTimers()
        // Defer first refresh so init doesn't block the main thread
        DispatchQueue.main.async { [weak self] in
            self?.refreshAll()
        }
    }

    deinit {
        refreshTimer?.invalidate()
        logTimer?.invalidate()
    }

    private func startTimers() {
        refreshTimer = Timer.scheduledTimer(withTimeInterval: 10, repeats: true) { [weak self] _ in
            Task { @MainActor [weak self] in
                self?.refreshAll()
            }
        }
        logTimer = Timer.scheduledTimer(withTimeInterval: 5, repeats: true) { [weak self] _ in
            Task { @MainActor [weak self] in
                await self?.loadLogTailAsync()
            }
        }
    }

    func refreshAll() {
        Task { @MainActor in
            await asyncRefreshAll()
        }
    }

    private func asyncRefreshAll() async {
        // Run all checks concurrently
        async let trios: Void = checkTriosAsync()
        async let mcp: Void = checkMCPAsync()
        async let agent: Void = checkAgentAsync()
        async let cron: Void = checkCronAsync()
        async let a2a: Void = checkA2AAsync()
        async let funnel: Void = checkFunnelAsync()
        async let git: Void = checkGitAsync()
        async let build: Void = checkBuildAsync()
        async let improve: Void = checkSelfImprovementAsync()

        _ = await (trios, mcp, agent, cron, a2a, funnel, git, build, improve)

        loadSkills()
        loadAgents()
        await loadLogTailAsync()
        computeOverallStatus()
    }

    // MARK: - Component Checks

    // MARK: - Async Component Checks

    private func checkTriosAsync() async {
        let running = await isTriosRunning()
        updateComponent(name: "TRIOS", icon: "macwindow", status: running ? .healthy : .down, detail: running ? "Running" : "Stopped", action: running ? nil : "Start")
    }

    /// Robust check: NSRunningApplication by bundle ID, then pgrep for both `trios` and `trios_app`.
    private func isTriosRunning() async -> Bool {
        // Method 1: NSRunningApplication (catches .app bundle launches where process name is `trios`)
        let apps = NSRunningApplication.runningApplications(withBundleIdentifier: "com.browseros.trios")
        if !apps.isEmpty { return true }

        // Method 2: pgrep for both possible binary names
        let r1 = await shellAsync("pgrep -x trios >/dev/null 2>&1 && echo 1 || echo 0") == "1"
        let r2 = await shellAsync("pgrep -x trios_app >/dev/null 2>&1 && echo 1 || echo 0") == "1"
        return r1 || r2
    }

    private func checkMCPAsync() async {
        let healthy = await shellAsync("curl -s -o /dev/null -w '%{http_code}' \(ProjectPaths.browserOSHealthURL) 2>/dev/null || echo 000") == "200"
        updateComponent(name: "MCP", icon: "server.rack", status: healthy ? .healthy : .down, detail: healthy ? "Online" : "Offline", action: healthy ? "Restart" : "Start")
    }

    private func checkAgentAsync() async {
        let healthy = await shellAsync("curl -s -o /dev/null -w '%{http_code}' \(ProjectPaths.agentHealthURL) 2>/dev/null || echo 000") == "200"
        updateComponent(name: "Agent", icon: "cpu", status: healthy ? .healthy : .down, detail: healthy ? "Online" : "Offline", action: healthy ? "Restart" : "Start")
    }

    private func checkCronAsync() async {
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

    private func checkA2AAsync() async {
        let agents = await shellAsync("ls \(ProjectPaths.claude("agents"))/*.md 2>/dev/null | wc -l | tr -d ' '")
        let count = Int(agents.trimmingCharacters(in: .whitespaces)) ?? 0
        let detail = count > 0 ? "\(count) agents" : "No agents"
        updateComponent(name: "A2A", icon: "network", status: count > 0 ? .healthy : .warning, detail: detail, action: nil)
    }

    private func checkFunnelAsync() async {
        let running = await shellAsync("pgrep -x tailscale >/dev/null 2>&1 && echo 1 || echo 0") == "1"
        updateComponent(name: "Funnel", icon: "globe", status: running ? .healthy : .warning, detail: running ? "Tailscale active" : "Not running", action: nil)
    }

    private func checkGitAsync() async {
        let branch = await shellAsync("cd \(projectRoot) && git branch --show-current 2>/dev/null || echo '—'")
        let dirty = await shellAsync("cd \(projectRoot) && git status --porcelain 2>/dev/null | wc -l | tr -d ' '")
        let dirtyCount = Int(dirty.trimmingCharacters(in: .whitespaces)) ?? 0
        let status: ComponentStatus = dirtyCount > 0 ? .warning : .healthy
        let detail = "\(branch.trimmingCharacters(in: .whitespaces)) · \(dirtyCount) dirty"
        updateComponent(name: "Git", icon: "arrow.triangle.branch", status: status, detail: detail, action: nil)
    }

    private func checkBuildAsync() async {
        let result = await shellAsync("cd \(projectRoot) && swiftc -typecheck main.swift rings/**/*.swift BR-OUTPUT/*.swift 2>&1 | head -5")
        let ok = result.trimmingCharacters(in: .whitespaces).isEmpty
        updateComponent(name: "Build", icon: "hammer", status: ok ? .healthy : .down, detail: ok ? "OK" : "Errors", action: nil)
    }

    private func checkSelfImprovementAsync() async {
        let fm = FileManager.default
        let auditDir = "\(projectRoot)/.trinity/audit"
        var findings = 0
        var lastAudit: Date? = nil
        var passed = true

        if let entries = try? fm.contentsOfDirectory(atPath: auditDir) {
            let jsons = entries.filter { $0.hasSuffix(".json") }
            findings = jsons.count
            for name in jsons {
                let path = "\(auditDir)/\(name)"
                if let attr = try? fm.attributesOfItem(atPath: path),
                   let mod = attr[.modificationDate] as? Date {
                    if lastAudit == nil || mod > lastAudit! {
                        lastAudit = mod
                    }
                }
            }
        }

        let budgetPath = "\(projectRoot)/.trinity/state/safety_budget.json"
        var budgetText = "—"
        if let data = try? Data(contentsOf: URL(fileURLWithPath: budgetPath)),
           let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
           let budget = json["budget"] as? Double,
           let halted = json["halted"] as? Bool {
            budgetText = halted ? "HALTED (\(budget))" : "\(budget)"
            if halted || budget <= 0 { passed = false }
        }

        let ago = lastAudit.map { timeAgo($0) } ?? "Never"
        let critique = findings > 0 ? "\(findings) audits on record" : "No audits yet"

        self.selfImprovement = SelfImprovementStatus(
            score: passed ? 100 : max(0, 100 - findings * 10),
            openIssues: findings,
            lastAuditAgo: ago,
            safetyBudget: budgetText,
            lastCritique: critique,
            pendingPRs: 0
        )

        if let last = lastAudit {
            auditHistory.append(AuditRecord(timestamp: last, findings: findings, passed: passed))
            if auditHistory.count > 10 {
                auditHistory.removeFirst(auditHistory.count - 10)
            }
        }
    }

    private func timeAgo(_ date: Date) -> String {
        let minutes = Int(Date().timeIntervalSince(date) / 60)
        if minutes < 1 { return "just now" }
        if minutes < 60 { return "\(minutes)m ago" }
        let hours = minutes / 60
        if hours < 24 { return "\(hours)h ago" }
        return "\(hours / 24)d ago"
    }

    private func loadLogTailAsync() async {
        let fm = FileManager.default
        guard fm.fileExists(atPath: logPath) else {
            lastLogLines = ["No cron log found"]
            return
        }
        let output = await shellAsync("tail -n 20 \(logPath)")
        lastLogLines = output.split(separator: "\n").map { String($0) }
        if lastLogLines.isEmpty {
            lastLogLines = ["Log empty"]
        }
    }

    // MARK: - Agent Management

    func loadAgents() {
        let agentNames = ["clade-monitor", "clade-dashboard", "cron-queen"]
        var result: [AgentInfo] = []
        for name in agentNames {
            let pid = Int(shell("pgrep -x \(name) 2>/dev/null || echo ''").trimmingCharacters(in: .whitespaces)) ?? nil
            let status: ComponentStatus = pid != nil ? .healthy : .down
            let uptime = pid != nil ? shell("ps -o etime= -p \(pid!) 2>/dev/null || echo 'unknown'").trimmingCharacters(in: .whitespaces) : "—"
            result.append(AgentInfo(
                name: name,
                status: status,
                pid: pid,
                uptime: uptime,
                lastAction: nil
            ))
        }
        agents = result
    }

    func startAgent(_ name: String) {
        let binMap: [String: (String, [String])] = [
            "clade-monitor": ("/usr/bin/env", ["cargo", "run", "--bin", "clade-monitor"]),
            "clade-dashboard": ("/usr/bin/env", ["cargo", "run", "--bin", "clade-dashboard"]),
        ]
        guard let (exe, args) = binMap[name] else {
            NSLog("[QueenStatus] BLOCKED: unknown agent: \(name)")
            return
        }
        isRunningAction = true
        execDirect(exe, arguments: args)
        DispatchQueue.main.asyncAfter(deadline: .now() + 2) { [weak self] in
            self?.loadAgents()
            self?.isRunningAction = false
        }
    }

    func stopAgent(_ name: String) {
        let knownAgents: Set<String> = ["clade-monitor", "clade-dashboard", "cron-queen"]
        guard knownAgents.contains(name) else {
            NSLog("[QueenStatus] BLOCKED: unknown agent name: \(name)")
            return
        }
        isRunningAction = true
        let task = Process()
        task.executableURL = URL(fileURLWithPath: "/usr/bin/pkill")
        task.arguments = ["-x", name]
        do {
            try task.run()
            task.waitUntilExit()
        } catch {
            NSLog("[QueenStatus] Failed to run pkill: \(error)")
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) { [weak self] in
            self?.loadAgents()
            self?.isRunningAction = false
        }
    }

    func restartAgent(_ name: String) {
        stopAgent(name)
        DispatchQueue.main.asyncAfter(deadline: .now() + 2) { [weak self] in
            self?.startAgent(name)
        }
    }

    // MARK: - Shell Helpers

    private func shellAsync(_ command: String) async -> String {
        await Task.detached {
            let task = Process()
            task.executableURL = URL(fileURLWithPath: "/bin/zsh")
            task.arguments = ["-c", command]
            task.currentDirectoryURL = URL(fileURLWithPath: self.projectRoot)

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
            return String(data: data, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        }.value
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

    // MARK: - Actions

    func startTrios() {
        Task { @MainActor in
            guard await !isTriosRunning() else {
                NSLog("[QueenStatus] startTrios blocked: TRIOS is already running")
                refreshAll()
                return
            }
            isRunningAction = true
            // Launch via the .app bundle so macOS single-instance semantics apply
            // and RecursionGuard/NSRunningApplication can detect duplicates.
            let bundlePath = "\(projectRoot)/trios.app"
            execDirect("/usr/bin/open", arguments: [bundlePath])
            DispatchQueue.main.asyncAfter(deadline: .now() + 3) { [weak self] in
                self?.refreshAll()
                self?.isRunningAction = false
            }
        }
    }

    func stopTrios() {
        isRunningAction = true
        // Kill both the bare binary and the .app bundle binary names.
        execDirect("/usr/bin/pkill", arguments: ["-9", "-f", "\(projectRoot)/trios.app/Contents/MacOS/trios"])
        execDirect("/usr/bin/pkill", arguments: ["-9", "trios_app"])
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) { [weak self] in
            self?.refreshAll()
            self?.isRunningAction = false
        }
    }

    func restartMCP() {
        isRunningAction = true
        execDirect("/usr/bin/pkill", arguments: ["-f", "bun.*start:server"])
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) { [weak self] in
            let bunPath = ProcessInfo.processInfo.environment["TRIOS_BUN_PATH"] ?? "/opt/homebrew/bin/bun"
            self?.execDirect(bunPath, arguments: ["run", "start:server"], workDir: self?.projectRoot)
            DispatchQueue.main.asyncAfter(deadline: .now() + 3) { [weak self] in
                self?.refreshAll()
                self?.isRunningAction = false
            }
        }
    }

    func restartAgentServer() {
        isRunningAction = true
        execDirect("/usr/bin/pkill", arguments: ["-f", "bun.*agent"])
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) { [weak self] in
            let bunPath = ProcessInfo.processInfo.environment["TRIOS_BUN_PATH"] ?? "/opt/homebrew/bin/bun"
            self?.execDirect(bunPath, arguments: ["run", "start:agent"], workDir: self?.projectRoot)
            DispatchQueue.main.asyncAfter(deadline: .now() + 3) { [weak self] in
                self?.refreshAll()
                self?.isRunningAction = false
            }
        }
    }

    func runCron() {
        isRunningAction = true
        execDirect("/usr/bin/env", arguments: ["cargo", "run", "--bin", "clade-monitor"])
        DispatchQueue.main.asyncAfter(deadline: .now() + 2) { [weak self] in
            self?.refreshAll()
            self?.isRunningAction = false
        }
    }

    private static let knownSkills: Set<String> = ["/tri", "/doctor", "/god-mode", "/bridge"]

    func runSkill(name: String) {
        guard Self.knownSkills.contains(name) else {
            NSLog("[QueenStatus] BLOCKED: unknown skill: \(name)")
            return
        }
        guard let index = skills.firstIndex(where: { $0.name == name }) else { return }
        skills[index] = SkillRun(name: name, lastRun: skills[index].lastRun, success: skills[index].success, isRunning: true)
        objectWillChange.send()

        execDirect("/usr/bin/env", arguments: ["claude", name])
        DispatchQueue.main.asyncAfter(deadline: .now() + 5) { [weak self] in
            if let idx = self?.skills.firstIndex(where: { $0.name == name }) {
                self?.skills[idx] = SkillRun(name: name, lastRun: Date(), success: true, isRunning: false)
                self?.objectWillChange.send()
            }
            self?.refreshAll()
        }
    }

    private static let commandAllowlist: [String] = [
        "git status", "git log", "git diff", "git branch",
        "cargo check", "cargo build", "cargo run --bin clade-",
        "curl -s http://127.0.0.1:", "swift --version",
        "cat .trinity/", "ls ", "wc ", "tail ", "head ",
        "pgrep", "ps aux"
    ]

    func runCommand(_ cmd: String) {
        let trimmed = cmd.trimmingCharacters(in: .whitespaces)
        // Literal (not regex) substring blocklist. The previous
        // `.regularExpression` match was unsafe: `$(` is an invalid pattern
        // (unclosed group) so it was never blocked, while `|`/`||` are regex
        // alternations matching the empty string. Literal `contains` matching
        // closes both. Per CWE-78 / OWASP A03 this blocklist over a shell is
        // bypassable (command/argument/wildcard injection), so it is
        // defense-in-depth only — the robust fix is shell-free `execDirect`.
        let blocked = [";", "&&", "||", "|", "`", "$(", "${", ">", "<", "..", "~", "rm -rf", "\n", "\r"]
        for b in blocked {
            // Literal substring match (no `.regularExpression` options).
            if trimmed.range(of: b) != nil {
                NSLog("[QueenStatus] BLOCKED dangerous token in command: \(b)")
                return
            }
        }
        let allowed = Self.commandAllowlist.contains { trimmed.hasPrefix($0) }
        guard allowed else {
            NSLog("[QueenStatus] BLOCKED unlisted command: \(trimmed)")
            return
        }
        isRunningAction = true
        // Shell-free execution: tokenise and run via /usr/bin/env, NOT
        // `/bin/zsh -c`. Shell metacharacters can no longer be interpreted —
        // at most they become literal argv — which structurally removes the
        // command-injection class (CWE-78 / OWASP A03). The blocklist/allowlist
        // above remain as defense-in-depth. PATH resolution is unchanged (env
        // inherits the same environment the old `zsh -c` did).
        let tokens = trimmed.split(separator: " ", omittingEmptySubsequences: true).map(String.init)
        guard !tokens.isEmpty else {
            isRunningAction = false
            return
        }
        let task = Process()
        task.executableURL = URL(fileURLWithPath: "/usr/bin/env")
        task.arguments = tokens
        task.currentDirectoryURL = URL(fileURLWithPath: projectRoot)
        do {
            try task.run()
        } catch {
            NSLog("[QueenStatus] Command failed: \(error)")
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) { [weak self] in
            Task { @MainActor [weak self] in
                await self?.loadLogTailAsync()
                self?.isRunningAction = false
            }
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

    private func execDirect(_ executable: String, arguments: [String], workDir: String? = nil) {
        let task = Process()
        task.executableURL = URL(fileURLWithPath: executable)
        task.arguments = arguments
        task.currentDirectoryURL = URL(fileURLWithPath: workDir ?? projectRoot)
        do {
            try task.run()
        } catch {
            NSLog("[QueenStatus] execDirect failed (\(executable)): \(error)")
        }
    }
}
