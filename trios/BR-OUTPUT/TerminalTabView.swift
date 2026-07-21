import SwiftUI

@MainActor
class TerminalViewModel: ObservableObject {
    @Published var output: String = "Trios Terminal — zsh ready\n"
    @Published var isRunning = false

    private var process: Process?
    private var pipe: Pipe?

    func runCommand(_ command: String) {
        isRunning = true
        output += "\n$ \(command)\n"

        let task = Process()
        task.executableURL = URL(fileURLWithPath: "/bin/zsh")
        task.arguments = ["-c", command]
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

    private static let blockedPatterns = [
        "trios_app", "open trios\\b", "open trios\\.app", "swiftc.*trios_app",
        "launchd.*trios", "clade-promote.*boot", "./trios"
    ]

    private static let dangerousChars = [";", "&&", "||", "`", "$(", ">{", "rm -rf", "curl.*|.*sh", "> /dev/null"]

    private func submit() {
        guard !command.isEmpty else { return }
        let cmd = command
        let lowerCmd = cmd.lowercased()

        for pattern in Self.blockedPatterns {
            if lowerCmd.range(of: pattern, options: .regularExpression) != nil {
                command = ""
                vm.output += "\n[BLOCKED] Recursive self-launch detected\n"
                return
            }
        }

        for pattern in Self.dangerousChars {
            if cmd.range(of: pattern, options: .regularExpression) != nil {
                command = ""
                vm.output += "\n[BLOCKED] Dangerous shell pattern rejected\n"
                return
            }
        }

        command = ""
        vm.runCommand(cmd)
    }
}
