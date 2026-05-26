import Foundation
import SwiftUI
import Combine

@MainActor
class BrowserOSChatViewModel: ObservableObject {
    
    @Published var messages: [BrowserOSChatMessage] = []
    @Published var isStreaming: Bool = false
    @Published var isBrowserOSConnected: Bool = false
    @Published var queenStatus: QueenStatus = .idle
    @Published var toolCalls: [ToolCallRecord] = []
    
    @Published var currentPageId: Int? = nil
    @Published var inputText: String = ""

    private let mcpClient: TriosMCPClient
    private var cancellables = Set<AnyCancellable>()
    private var streamingTask: Task<Void, Never>?
    private var sessionStartTime: Date = Date()
    private var pageDetectionTask: Task<Void, Never>?

    enum QueenStatus: String {
        case idle = "idle"
        case alive = "alive"
        case working = "working"
        case error = "error"
    }
    
    struct ToolCallRecord: Identifiable {
        let id = UUID()
        let name: String
        let status: ToolStatus
        let timestamp: Date
        let result: String?
        
        enum ToolStatus {
            case running, completed, failed
        }
    }
    
    init() {
        self.mcpClient = TriosMCPClient()
        setupHealthCheck()
    }
    
    private func setupHealthCheck() {
        Task {
            _ = await mcpClient.checkHealth()
            Timer.publish(every: 5, on: .main, in: .common)
                .autoconnect()
                .sink { [weak self] _ in
                    Task {
                        await self?.updateConnectionStatus()
                    }
                }
                .store(in: &cancellables)
        }
    }
    
    private func updateConnectionStatus() async {
        let connected = await mcpClient.checkHealth()
        self.isBrowserOSConnected = connected
        if connected && queenStatus == .idle {
            self.queenStatus = .alive
        } else if !connected {
            self.queenStatus = .error
        }
    }
    
    func sendMessage(_ text: String) {
        let userMessage = BrowserOSChatMessage(role: .user, content: text, timestamp: Date())
        messages.append(userMessage)
        if isLikelyCommand(text) {
            executeBrowserOSCommand(text)
        } else {
            showUsageHint()
        }
    }

    func isLikelyCommand(_ text: String) -> Bool {
        let lower = text.lowercased()
        let commands = ["navigate", "click", "screenshot", "extract", "open", "go to", "browse",
                        "shell", "run", "exec", "ls", "pwd", "cd", "cat", "mkdir", "rm", "git",
                        "curl", "wget", "npm", "bun", "node", "python", "swift"]
        return commands.contains { lower.contains($0) } || lower.hasPrefix("/") || lower.hasPrefix("./")
    }

    private func showUsageHint() {
        isStreaming = true
        let response = """
        BrowserOS Agent ready. Available commands:
        • open [url] — navigate to page
        • click — click element
        • screenshot — capture page
        • extract — get page content
        • shell [command] — run shell command
        """
        let agentMessage = BrowserOSChatMessage(
            role: .assistant,
            content: response,
            timestamp: Date()
        )
        messages.append(agentMessage)
        isStreaming = false
    }
    
    private func executeBrowserOSCommand(_ text: String) {
        isStreaming = true
        queenStatus = .working

        streamingTask = Task {
            do {
                // Auto-detect page ID before executing browser tools
                let pageId = await ensurePageId()

                let (toolName, args) = parseIntent(text, pageId: pageId)

                let record = ToolCallRecord(
                    name: toolName,
                    status: .running,
                    timestamp: Date(),
                    result: nil
                )
                toolCalls.append(record)

                let response = try await mcpClient.callTool(
                    name: toolName,
                    arguments: args
                )

                let resultText = extractResultText(response)

                if let index = toolCalls.lastIndex(where: { $0.name == toolName }) {
                    toolCalls[index] = ToolCallRecord(
                        name: toolName,
                        status: .completed,
                        timestamp: toolCalls[index].timestamp,
                        result: resultText
                    )
                }

                let agentMessage = BrowserOSChatMessage(
                    role: .assistant,
                    content: resultText,
                    timestamp: Date(),
                    toolCalls: [BrowserOSToolCall(name: toolName, result: resultText)]
                )
                messages.append(agentMessage)

                queenStatus = .alive

            } catch {
                let errorMessage = BrowserOSChatMessage(
                    role: .system,
                    content: "BrowserOS Error: \(error.localizedDescription)",
                    timestamp: Date()
                )
                messages.append(errorMessage)
                queenStatus = .error
            }

            isStreaming = false
        }
    }

    private func parseIntent(_ text: String, pageId: Int?) -> (String, [String: Any]) {
        let lower = text.lowercased()

        if lower.contains("navigate") || lower.contains("go to") || lower.contains("open") {
            let url = extractURL(from: text) ?? "https://google.com"
            var args: [String: Any] = ["url": url]
            if let pageId = pageId { args["page"] = pageId }
            return ("navigate_page", args)
        }

        if lower.contains("click") || lower.contains("press") {
            var args: [String: Any] = ["element": "1"]
            if let pageId = pageId { args["page"] = pageId }
            return ("click", args)
        }

        if lower.contains("screenshot") || lower.contains("capture") {
            var args: [String: Any] = [:]
            if let pageId = pageId { args["page"] = pageId }
            return ("take_screenshot", args)
        }

        if lower.contains("extract") || lower.contains("get data") || lower.contains("content") {
            var args: [String: Any] = [:]
            if let pageId = pageId { args["page"] = pageId }
            return ("get_page_content", args)
        }

        if lower.hasPrefix("shell ") || lower.hasPrefix("run ") || lower.hasPrefix("exec ") {
            let cmd = String(text.dropFirst(lower.hasPrefix("shell ") ? 6 : (lower.hasPrefix("run ") ? 4 : 5)))
            return ("filesystem_bash", ["command": cmd, "description": "User shell command"])
        }

        return ("filesystem_bash", ["command": text, "description": "User command"])
    }

    private func ensurePageId() async -> Int? {
        if let cached = currentPageId { return cached }
        return await detectPageId()
    }

    private func detectPageId() async -> Int? {
        do {
            let pagesJSON = try await mcpClient.listPages()
            // Parse JSON array of pages, extract first page ID
            if let data = pagesJSON.data(using: .utf8),
               let pages = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]],
               let first = pages.first,
               let id = first["id"] as? Int {
                currentPageId = id
                return id
            }
            // Fallback: try parsing as single object
            if let data = pagesJSON.data(using: .utf8),
               let page = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
               let id = page["id"] as? Int {
                currentPageId = id
                return id
            }
        } catch {
            NSLog("[BrowserOSChatViewModel] Page detection failed: \(error)")
        }
        return nil
    }

    func startPageDetection() {
        pageDetectionTask?.cancel()
        pageDetectionTask = Task {
            while !Task.isCancelled {
                if currentPageId == nil {
                    _ = await detectPageId()
                }
                try? await Task.sleep(nanoseconds: 10_000_000_000) // 10 seconds
            }
        }
    }

    func stopPageDetection() {
        pageDetectionTask?.cancel()
        pageDetectionTask = nil
    }
    
    private func extractURL(from text: String) -> String? {
        let pattern = #"(https?://[^\s]+)"#
        guard let regex = try? NSRegularExpression(pattern: pattern) else { return nil }
        let range = NSRange(text.startIndex..., in: text)
        if let match = regex.firstMatch(in: text, range: range) {
            return String(text[Range(match.range, in: text)!])
        }
        return nil
    }
    
    private func extractResultText(_ response: MCPResponse) -> String {
        guard let result = response.result else { return "No result" }
        return result.content.compactMap { $0.text }.joined(separator: "\n")
    }
    
    var sessionDuration: String {
        let interval = Date().timeIntervalSince(sessionStartTime)
        let minutes = Int(interval) / 60
        return minutes > 0 ? "\(minutes)m" : "\(Int(interval))s"
    }
    
    var queenStatusText: String {
        "👑 \(queenStatus.rawValue) \(sessionDuration)"
    }
}

struct BrowserOSChatMessage: Identifiable {
    let id = UUID()
    let role: ChatRole
    let content: String
    let timestamp: Date
    var toolCalls: [BrowserOSToolCall] = []
    
    enum ChatRole {
        case user, assistant, system, tool
    }
}

struct BrowserOSToolCall {
    let name: String
    let result: String?
}