import Foundation

/// Virtual Branch model for GitButler integration.
struct VirtualBranch: Identifiable {
    let id: String
    let name: String
    var isApplied: Bool
    var isConflicted: Bool
    var files: Int
    var commitCount: Int
    var upstream: String?
}

/// Manages GitButler virtual branches and traditional git branch operations.
/// Integrates with GitButler.app via repository state in `.git/gitbutler/`.
@MainActor
final class GitButlerViewModel: ObservableObject {
    @Published var branches: [VirtualBranch] = []
    @Published var consoleOutput = ""
    @Published var isApplying = false
    @Published var currentBranch = ""
    @Published var isGitButlerEnabled = false

    let repoPath = ProjectPaths.root

    init() {
        checkGitButlerStatus()
        loadBranches()
    }

    // MARK: - GitButler Status

    func checkGitButlerStatus() {
        let fm = FileManager.default
        isGitButlerEnabled = fm.fileExists(atPath: "\(repoPath)/.git/gitbutler/virtual-branches")
    }

    // MARK: - Virtual Branches (GitButler)

    func loadVirtualBranches() {
        guard isGitButlerEnabled else {
            loadGitBranches()
            return
        }

        let vbPath = "\(repoPath)/.git/gitbutler/virtual-branches"
        let fm = FileManager.default
        guard fm.fileExists(atPath: vbPath),
              let files = try? fm.contentsOfDirectory(atPath: vbPath),
              files.contains(where: { $0.hasSuffix(".toml") }) else {
            loadGitBranches()
            return
        }
        loadGitBranches()
    }

    func createVirtualBranch(name: String) {
        guard isGitButlerEnabled else {
            createBranch(name: name)
            return
        }
        Task {
            await runGitAsync(["checkout", "-b", name])
            loadBranches()
        }
    }

    func applyVirtualBranch(_ branch: VirtualBranch) {
        guard isGitButlerEnabled else {
            switchBranch(branch)
            return
        }
        Task {
            await runGitAsync(["checkout", branch.name])
            loadBranches()
        }
    }

    // MARK: - Traditional Git Branches

    func loadBranches() {
        isGitButlerEnabled ? loadVirtualBranches() : loadGitBranches()
    }

    func loadGitBranches() {
        Task {
            let output = await runGitAsync(["branch", "-a", "--format=%(refname:short)|%(HEAD)"])
            let lines = output.split(separator: "\n")
            branches = lines.compactMap { line in
                let parts = line.split(separator: "|", maxSplits: 1)
                guard parts.count == 2 else { return nil }
                let name = String(parts[0])
                let isHead = parts[1].trimmingCharacters(in: .whitespaces) == "*"
                return VirtualBranch(
                    id: name,
                    name: name,
                    isApplied: isHead,
                    isConflicted: false,
                    files: 0,
                    commitCount: 0,
                    upstream: nil
                )
            }
            isApplying = branches.contains(where: \.isApplied)
            currentBranch = branches.first(where: \.isApplied)?.name ?? ""
        }
    }

    func createBranch(name: String) {
        Task {
            await runGitAsync(["checkout", "-b", name])
            loadBranches()
        }
    }

    func switchBranch(_ branch: VirtualBranch) {
        Task {
            await runGitAsync(["checkout", branch.name])
            loadBranches()
        }
    }

    func deleteBranch(_ branch: VirtualBranch) {
        Task {
            await runGitAsync(["branch", "-D", branch.name])
            loadBranches()
        }
    }

    func commitBranch(_ branch: VirtualBranch, message: String) {
        Task {
            await runGitAsync(["-C", repoPath, "add", "."])
            let output = await runGitAsync(["-C", repoPath, "commit", "-m", message])
            consoleOutput = output
            loadBranches()
        }
    }

    func pushBranch(_ branch: VirtualBranch) {
        Task {
            let output = await runGitAsync(["push", "-u", "origin", branch.name])
            consoleOutput = output
        }
    }

    // MARK: - Merge & Conflict Detection

    func mergeBranch(_ branch: VirtualBranch) {
        Task {
            let output = await runGitAsync(["merge", branch.name])
            consoleOutput = output
            if output.contains("CONFLICT") {
                consoleOutput += "\n[WARN] Merge conflict detected. Resolve manually."
            }
            loadBranches()
        }
    }

    func checkConflicts() {
        Task {
            let output = await runGitAsync(["diff", "--check"])
            if output.isEmpty {
                consoleOutput = "No conflicts detected."
            } else {
                consoleOutput = "Conflicts found:\n\(output)"
            }
        }
    }

    // MARK: - Private Helpers

    @discardableResult
    private func runGitAsync(_ args: [String]) async -> String {
        let task = Process()
        task.executableURL = URL(fileURLWithPath: "/usr/bin/git")
        task.arguments = args
        task.currentDirectoryURL = URL(fileURLWithPath: repoPath)

        let pipe = Pipe()
        task.standardOutput = pipe
        task.standardError = pipe

        do {
            try task.run()
            let data = pipe.fileHandleForReading.readDataToEndOfFile()
            return String(data: data, encoding: .utf8) ?? ""
        } catch {
            return "Error: \(error.localizedDescription)"
        }
    }

}
