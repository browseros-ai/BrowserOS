import SwiftUI

/// Errors thrown by the terminal command sanitizer.
enum TerminalCommandError: Error {
    case emptyCommand
    case unknownExecutable(String)
    case tooManyArguments(Int)
    case blockedPattern(String)
    case missingExecutable
}

@MainActor
class TerminalViewModel: ObservableObject {
    @Published var output: String = "Trios Terminal - tokenized command runner\n"
    @Published var isRunning = false

    private var process: Process?
    private var pipe: Pipe?

    /// Runs a user-typed command after tokenization and allowlist validation.
    /// Never invokes a shell.
    func runCommand(_ command: String) {
        isRunning = true
        output += "\n$ \(command)\n"

        do {
            let request = try TerminalCommandSanitizer.sanitize(command)
            runTokenized(request)
        } catch TerminalCommandError.blockedPattern(let pattern) {
            output += "[BLOCKED] Recursive self-launch or forbidden pattern: \(pattern)\n"
            isRunning = false
        } catch TerminalCommandError.unknownExecutable(let exe) {
            output += "[BLOCKED] Unknown or disallowed executable: \(exe)\n"
            isRunning = false
        } catch TerminalCommandError.tooManyArguments(let count) {
            output += "[BLOCKED] Too many arguments (\(count)); split into smaller commands\n"
            isRunning = false
        } catch {
            output += "[BLOCKED] Invalid command: \(error.localizedDescription)\n"
            isRunning = false
        }
    }

    private func runTokenized(_ request: TerminalRequest) {
        let task = Process()
        task.executableURL = URL(fileURLWithPath: request.executable)
        task.arguments = request.arguments
        task.currentDirectoryURL = URL(fileURLWithPath: ProjectPaths.root)

        let outPipe = Pipe()
        let errPipe = Pipe()
        task.standardOutput = outPipe
        task.standardError = errPipe

        outPipe.fileHandleForReading.readabilityHandler = { [weak self] handle in
            let data = handle.availableData
            guard let str = String(data: data, encoding: .utf8), !str.isEmpty else { return }
            Task { @MainActor in
                self?.output += str
            }
        }

        errPipe.fileHandleForReading.readabilityHandler = { [weak self] handle in
            let data = handle.availableData
            guard let str = String(data: data, encoding: .utf8), !str.isEmpty else { return }
            Task { @MainActor in
                self?.output += str
            }
        }

        task.terminationHandler = { [weak self] _ in
            Task { @MainActor in
                self?.isRunning = false
            }
        }

        self.process = task
        self.pipe = outPipe

        do {
            try task.run()
        } catch {
            output += "Error: \(error.localizedDescription)\n"
            isRunning = false
        }
    }

    func kill() {
        process?.terminate()
        isRunning = false
    }

    func clear() {
        output = ""
    }
}

/// A sanitized terminal request: absolute executable path + literal arguments.
struct TerminalRequest {
    let executable: String
    let arguments: [String]
}

/// Shell-free command sanitizer for the in-app terminal.
/// Uses an explicit allowlist of executables and argument patterns.
enum TerminalCommandSanitizer {
    /// Maximum number of arguments accepted for a single command.
    static let maxArguments = 32

    /// Allowed executables mapped to their absolute paths.
    static let allowedExecutables: [String: String] = [
        "git": "/usr/bin/git",
        "ls": "/bin/ls",
        "cat": "/bin/cat",
        "pwd": "/bin/pwd",
        "whoami": "/usr/bin/whoami",
        "ps": "/bin/ps",
        "pgrep": "/usr/bin/pgrep",
        "pkill": "/usr/bin/pkill",
        "tail": "/usr/bin/tail",
        "head": "/usr/bin/head",
        "wc": "/usr/bin/wc",
        "echo": "/bin/echo",
        "swiftc": "/usr/bin/swiftc",
        "cargo": "/usr/bin/cargo",
        "bun": "/opt/homebrew/bin/bun",
        "node": "/opt/homebrew/bin/node",
        "npm": "/opt/homebrew/bin/npm",
        "python3": "/usr/bin/python3",
        "python": "/usr/bin/python3",
        "mkdir": "/bin/mkdir",
        "touch": "/usr/bin/touch",
        "rm": "/bin/rm",
        "cp": "/bin/cp",
        "mv": "/bin/mv",
        "open": "/usr/bin/open",
        "kill": "/bin/kill",
        "top": "/usr/bin/top",
        "clear": "/usr/bin/clear",
        "env": "/usr/bin/env",
        "date": "/bin/date",
        "uname": "/usr/bin/uname",
        "df": "/bin/df",
        "du": "/usr/bin/du",
        "find": "/usr/bin/find",
        "grep": "/usr/bin/grep",
        "awk": "/usr/bin/awk",
        "sed": "/usr/bin/sed"
    ]

    /// Shell metacharacters and substitutions that are never allowed in a token.
    static let forbiddenCharacters = CharacterSet(charactersIn: ";|&$`<>()\\\n")

    /// Full-command substrings that indicate recursive self-launch or other abuse.
    static let blockedSubstrings = [
        "trios_app", "trios.app", "open trios", "open trios.app",
        "launchd", "clade-promote", "./trios",
        ">/dev/null", "rm -rf /", "$(", ";", "&&", "||"
    ]

    /// Sanitizes a raw user-typed command.
    /// - Returns a `TerminalRequest` on success.
    /// - Throws `TerminalCommandError` for disallowed input.
    static func sanitize(_ command: String) throws -> TerminalRequest {
        let trimmed = command.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            throw TerminalCommandError.emptyCommand
        }

        // Reject shell metacharacters anywhere in the full input before tokenizing.
        if let bad = trimmed.unicodeScalars.first(where: { forbiddenCharacters.contains($0) }) {
            throw TerminalCommandError.blockedPattern(String(bad))
        }

        // Reject multi-token abuse patterns against the whole command string.
        let lowerCommand = trimmed.lowercased()
        for pattern in blockedSubstrings {
            if lowerCommand.contains(pattern) {
                throw TerminalCommandError.blockedPattern(pattern)
            }
        }

        // Split on whitespace only. This is intentionally simple: no shell quoting,
        // no variable expansion, no command substitution. Each token becomes a literal argv.
        let tokens = trimmed.split(separator: " ", omittingEmptySubsequences: true).map(String.init)
        guard let first = tokens.first else {
            throw TerminalCommandError.emptyCommand
        }

        guard tokens.count <= maxArguments else {
            throw TerminalCommandError.tooManyArguments(tokens.count)
        }

        // Resolve executable. Accept either a bare name from the allowlist or an
        // absolute path that matches one of the allowed absolute paths.
        let executable: String
        if let absolute = allowedExecutables[first] {
            executable = absolute
        } else if first.hasPrefix("/"), allowedExecutables.values.contains(first) {
            executable = first
        } else {
            throw TerminalCommandError.unknownExecutable(first)
        }

        let arguments = Array(tokens.dropFirst())

        // Final per-token forbidden-character check in case whitespace splitting hid anything.
        for token in tokens {
            if let bad = token.unicodeScalars.first(where: { forbiddenCharacters.contains($0) }) {
                throw TerminalCommandError.blockedPattern(String(bad))
            }
        }

        return TerminalRequest(executable: executable, arguments: arguments)
    }
}

struct TerminalTabView: View {
    @StateObject private var vm = TerminalViewModel()
    @State private var command: String = ""
    @State private var scrollToBottom = UUID()

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider().overlay(Color.grokBorder)
            outputArea
            Divider().overlay(Color.grokBorder)
            inputBar
        }
        .background(Color.clear)
    }

    private var header: some View {
        HStack(spacing: 8) {
            Text("Terminal")
                .font(.system(size: 13, weight: .semibold))
                .foregroundColor(.grokText)
            Spacer()
            if vm.isRunning {
                ProgressView()
                    .scaleEffect(0.6)
            }
            Button(action: { vm.clear() }) {
                Image(systemName: "trash")
                    .font(.system(size: 11))
                    .foregroundColor(.grokMuted)
            }
            .buttonStyle(.plain)
            Button(action: { vm.kill() }) {
                Image(systemName: "stop.fill")
                    .font(.system(size: 11))
                    .foregroundColor(.grokMuted)
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }

    private var outputArea: some View {
        ScrollViewReader { proxy in
            ScrollView {
                Text(vm.output)
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundColor(.grokMuted)
                    .padding(8)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .id(scrollToBottom)
            }
            .onChange(of: vm.output) {
                scrollToBottom = UUID()
                DispatchQueue.main.async {
                    withAnimation {
                        proxy.scrollTo(scrollToBottom, anchor: .bottom)
                    }
                }
            }
        }
        .background(Color.grokElevated.opacity(0.3))
    }

    private var inputBar: some View {
        HStack(spacing: 8) {
            TextField("Run command...", text: $command)
                .font(.system(size: 12, design: .monospaced))
                .foregroundColor(.grokText)
                .textFieldStyle(PlainTextFieldStyle())
                .onSubmit {
                    submit()
                }

            Button(action: submit) {
                Image(systemName: "return")
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundColor(.grokAccent)
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(Color.grokElevated.opacity(0.3))
    }

    private func submit() {
        guard !command.isEmpty else { return }
        let cmd = command
        command = ""
        vm.runCommand(cmd)
    }
}
