import SwiftUI

struct QueenTabView: View {
    @StateObject private var viewModel = QueenStatusViewModel()
    @State private var commandText: String = ""
    @State private var showLog = true
    @State private var scrollToBottom = UUID()

    var body: some View {
        VStack(spacing: 0) {
            headerBar
            Divider().overlay(Color.grokBorder)
            ScrollView {
                LazyVStack(spacing: 12) {
                    healthGrid
                    selfImprovementSection
                    skillsSection
                    logSection
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 12)
            }
            Divider().overlay(Color.grokBorder)
            commandBar
        }
        .background(Color.clear)
    }

    // MARK: - Header

    private var headerBar: some View {
        HStack(spacing: 8) {
            Image(systemName: "crown.fill")
                .font(.system(size: 13, weight: .semibold))
                .foregroundColor(headerIconColor)
            Text("Queen Trinity")
                .font(.system(size: 13, weight: .semibold))
                .foregroundColor(.grokText)
            Spacer()
            if viewModel.isRunningAction {
                ProgressView()
                    .scaleEffect(0.6)
            }
            Button(action: { viewModel.refreshAll() }) {
                Image(systemName: "arrow.clockwise")
                    .font(.system(size: 11))
                    .foregroundColor(.grokMuted)
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }

    private var headerIconColor: Color {
        switch viewModel.overallStatus {
        case .healthy: return .grokAccent
        case .warning: return Color.yellow
        case .down: return Color.red
        case .unknown: return .grokDim
        }
    }

    // MARK: - Health Grid

    private var healthGrid: some View {
        let columns = [GridItem(.flexible()), GridItem(.flexible())]
        return LazyVGrid(columns: columns, spacing: 8) {
            ForEach(viewModel.components) { comp in
                healthCard(comp)
            }
        }
    }

    private func healthCard(_ comp: StatusComponent) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 6) {
                Image(systemName: comp.icon)
                    .font(.system(size: 14))
                    .foregroundColor(.grokMuted)
                Text(comp.name)
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundColor(.grokText)
                Spacer()
                Circle()
                    .fill(statusColor(comp.status))
                    .frame(width: 8, height: 8)
            }

            Text(comp.detail)
                .font(.system(size: 10))
                .foregroundColor(.grokMuted)
                .lineLimit(1)

            if let action = comp.actionLabel {
                HStack {
                    Spacer()
                    Button(action: {
                        runAction(for: comp.name)
                    }) {
                        Text(action)
                            .font(.system(size: 10, weight: .semibold))
                            .foregroundColor(.grokAccent)
                            .padding(.horizontal, 8)
                            .padding(.vertical, 3)
                            .background(Color.grokElevated.opacity(0.6))
                            .cornerRadius(4)
                    }
                    .buttonStyle(.plain)
                }
            }
        }
        .padding(10)
        .background(Color.grokElevated.opacity(0.25))
        .cornerRadius(10)
        .overlay(
            RoundedRectangle(cornerRadius: 10)
                .stroke(Color.grokBorder.opacity(0.3), lineWidth: 1)
        )
    }

    // MARK: - Self-Improvement Section

    private var selfImprovementSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("Self-Critic")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundColor(.grokMuted)
                Spacer()
                if let status = viewModel.selfImprovement {
                    Text("Score: \(status.score)")
                        .font(.system(size: 11, weight: .semibold))
                        .foregroundColor(status.score >= 80 ? Color.green : (status.score >= 50 ? Color.yellow : Color.red))
                }
            }

            if let status = viewModel.selfImprovement {
                VStack(alignment: .leading, spacing: 6) {
                    HStack(spacing: 12) {
                        metricChip("Issues", value: "\(status.openIssues)")
                        metricChip("Budget", value: status.safetyBudget)
                        metricChip("Pending PRs", value: "\(status.pendingPRs)")
                    }

                    HStack {
                        Text(status.lastCritique)
                            .font(.system(size: 10))
                            .foregroundColor(.grokDim)
                        Spacer()
                        Text(status.lastAuditAgo)
                            .font(.system(size: 9))
                            .foregroundColor(.grokDim)
                    }

                    HStack(spacing: 8) {
                        Button(action: { viewModel.runCommand("cargo run --bin clade-audit -- --json") }) {
                            Text("Run Audit")
                                .font(.system(size: 10, weight: .semibold))
                                .foregroundColor(.grokAccent)
                                .padding(.horizontal, 8)
                                .padding(.vertical, 4)
                                .background(Color.grokElevated.opacity(0.6))
                                .cornerRadius(4)
                        }
                        .buttonStyle(.plain)

                        Button(action: { viewModel.runCommand("cargo run --bin clade-audit -- generate-awareness") }) {
                            Text("Awareness")
                                .font(.system(size: 10, weight: .semibold))
                                .foregroundColor(.grokAccent)
                                .padding(.horizontal, 8)
                                .padding(.vertical, 4)
                                .background(Color.grokElevated.opacity(0.6))
                                .cornerRadius(4)
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(10)
                .background(Color.grokElevated.opacity(0.2))
                .cornerRadius(8)
                .overlay(
                    RoundedRectangle(cornerRadius: 8)
                        .stroke(Color.grokBorder.opacity(0.3), lineWidth: 1)
                )
            } else {
                Text("No self-improvement data yet. Run audit to populate.")
                    .font(.system(size: 10))
                    .foregroundColor(.grokDim)
                    .padding(10)
                    .background(Color.grokElevated.opacity(0.2))
                    .cornerRadius(8)
            }
        }
    }

    private func metricChip(_ label: String, value: String) -> some View {
        VStack(spacing: 2) {
            Text(value)
                .font(.system(size: 12, weight: .semibold))
                .foregroundColor(.grokText)
            Text(label)
                .font(.system(size: 9))
                .foregroundColor(.grokDim)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 6)
        .background(Color.grokElevated.opacity(0.3))
        .cornerRadius(6)
    }

    // MARK: - Skills Section

    private var skillsSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Skills")
                .font(.system(size: 11, weight: .semibold))
                .foregroundColor(.grokMuted)

            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(viewModel.skills) { skill in
                        skillChip(skill)
                    }
                }
            }
        }
    }

    private func skillChip(_ skill: SkillRun) -> some View {
        Button(action: {
            viewModel.runSkill(name: skill.name)
        }) {
            HStack(spacing: 4) {
                if skill.isRunning {
                    ProgressView()
                        .scaleEffect(0.5)
                } else if let success = skill.success {
                    Image(systemName: success ? "checkmark" : "xmark")
                        .font(.system(size: 8, weight: .bold))
                        .foregroundColor(success ? Color.green : Color.red)
                }
                Text(skill.name)
                    .font(.system(size: 11, weight: .medium))
                    .foregroundColor(.grokText)
                if let last = skill.lastRun {
                    Text(" /  \(timeAgo(last))")
                        .font(.system(size: 9))
                        .foregroundColor(.grokDim)
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .background(Color.grokElevated.opacity(0.3))
            .cornerRadius(8)
            .overlay(
                RoundedRectangle(cornerRadius: 8)
                    .stroke(Color.grokBorder.opacity(0.3), lineWidth: 1)
            )
        }
        .buttonStyle(.plain)
        .disabled(skill.isRunning)
    }

    // MARK: - Log Section

    private var logSection: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text("Queen Log")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundColor(.grokMuted)
                Spacer()
                Button(action: { showLog.toggle() }) {
                    Image(systemName: showLog ? "chevron.up" : "chevron.down")
                        .font(.system(size: 10))
                        .foregroundColor(.grokMuted)
                }
                .buttonStyle(.plain)
            }

            if showLog {
                ScrollViewReader { proxy in
                    ScrollView {
                        VStack(alignment: .leading, spacing: 2) {
                            ForEach(viewModel.lastLogLines.indices, id: \.self) { i in
                                Text(viewModel.lastLogLines[i])
                                    .font(.system(size: 9, design: .monospaced))
                                    .foregroundColor(.grokDim)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                            }
                        }
                        .id(scrollToBottom)
                    }
                    .frame(height: 120)
                    .background(Color.grokElevated.opacity(0.2))
                    .cornerRadius(8)
                    .onChange(of: viewModel.lastLogLines.count) {
                        scrollToBottom = UUID()
                        DispatchQueue.main.async {
                            withAnimation {
                                proxy.scrollTo(scrollToBottom, anchor: .bottom)
                            }
                        }
                    }
                }
            }
        }
    }

    // MARK: - Command Bar

    private var commandBar: some View {
        VStack(spacing: 6) {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 6) {
                    presetButton("git status")
                    presetButton("git pull")
                    presetButton("bun run start:server")
                }
            }
            HStack(spacing: 8) {
                TextField("Run command...", text: $commandText)
                    .font(.system(size: 12, design: .monospaced))
                    .foregroundColor(.grokText)
                    .textFieldStyle(PlainTextFieldStyle())
                    .onSubmit {
                        submitCommand()
                    }
                Button(action: submitCommand) {
                    Image(systemName: "return")
                        .font(.system(size: 12, weight: .semibold))
                        .foregroundColor(.grokAccent)
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(Color.grokElevated.opacity(0.2))
    }

    private func presetButton(_ cmd: String) -> some View {
        Button(action: {
            commandText = cmd
            submitCommand()
        }) {
            Text(cmd)
                .font(.system(size: 9, design: .monospaced))
                .foregroundColor(.grokMuted)
                .padding(.horizontal, 8)
                .padding(.vertical, 3)
                .background(Color.grokElevated.opacity(0.4))
                .cornerRadius(4)
        }
        .buttonStyle(.plain)
    }

    // MARK: - Helpers

    private func statusColor(_ status: ComponentStatus) -> Color {
        switch status {
        case .healthy: return Color.green
        case .warning: return Color.yellow
        case .down: return Color.red
        case .unknown: return Color.grokDim
        }
    }

    private func runAction(for name: String) {
        switch name {
        case "TRIOS": viewModel.startTrios()
        case "MCP": viewModel.restartMCP()
        case "Agent": viewModel.restartAgentServer()
        case "Mesh": viewModel.restartAgent("clade-meshd")
        case "Cron": viewModel.runCron()
        default: break
        }
    }

    private func submitCommand() {
        guard !commandText.isEmpty else { return }
        let cmd = commandText

        // SAFETY: Block commands that would recursively launch trios
        let lowerCmd = cmd.lowercased()
        let blockedPatterns = ["./trios_app", "trios_app", "open trios\\b", "open trios\\.app", "swiftc.*trios_app", "launchd.*trios"]
        for pattern in blockedPatterns {
            if lowerCmd.range(of: pattern, options: .regularExpression) != nil {
                commandText = ""
                viewModel.runCommand("echo 'Blocked: recursive self-launch detected: \(cmd)'")
                return
            }
        }

        commandText = ""
        viewModel.runCommand(cmd)
    }

    private func timeAgo(_ date: Date) -> String {
        let minutes = Int(Date().timeIntervalSince(date) / 60)
        if minutes < 1 { return "just now" }
        if minutes < 60 { return "\(minutes)m ago" }
        let hours = minutes / 60
        if hours < 24 { return "\(hours)h ago" }
        return "\(hours / 24)d ago"
    }
}
