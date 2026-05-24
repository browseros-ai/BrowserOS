import Foundation

@MainActor
class GitButlerViewModel: ObservableObject {
    @Published var branches: [VirtualBranch] = []
    @Published var consoleOutput = ""
    @Published var isApplying = false
    @Published var currentBranch = ""

    let repoPath = ProjectPaths.root

    init() {
        loadBranches()
    }

    func loadBranches() {
        runGit(["branch", "-a", "--format=%(refname:short)|%(HEAD)"]) { output in
            let lines = output.split(separator: "\n")
            self.branches = lines.compactMap { line in
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
            self.isApplying = self.branches.contains(where: \.isApplied)
            self.currentBranch = self.branches.first(where: \.isApplied)?.name ?? ""
        }
    }

    func createBranch(name: String) {
        runGit(["checkout", "-b", name]) { _ in
            self.loadBranches()
        }
    }

    func switchBranch(_ branch: VirtualBranch) {
        runGit(["checkout", branch.name]) { _ in
            self.loadBranches()
        }
    }

    func deleteBranch(_ branch: VirtualBranch) {
        runGit(["branch", "-D", branch.name]) { _ in
            self.loadBranches()
        }
    }

    func commitBranch(_ branch: VirtualBranch, message: String) {
        runGit(["-C", repoPath, "add", "."]) { _ in
            self.runGit(["-C", self.repoPath, "commit", "-m", message]) { output in
                self.consoleOutput = output
                self.loadBranches()
            }
        }
    }

    func pushBranch(_ branch: VirtualBranch) {
        runGit(["push", "-u", "origin", branch.name]) { output in
            self.consoleOutput = output
        }
    }

    private func runGit(_ args: [String], completion: @escaping @MainActor (String) -> Void) {
        let task = Process()
        task.executableURL = URL(fileURLWithPath: "/usr/bin/git")
        task.arguments = args
        task.currentDirectoryURL = URL(fileURLWithPath: repoPath)

        let pipe = Pipe()
        task.standardOutput = pipe
        task.standardError = pipe

        Task {
            do {
                try task.run()
                let data = pipe.fileHandleForReading.readDataToEndOfFile()
                let output = String(data: data, encoding: .utf8) ?? ""
                await completion(output)
            } catch {
                await completion("Error: \(error.localizedDescription)")
            }
        }
    }
}
