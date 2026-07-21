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
    private let llmClient = LLMClient()
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
            if let (toolName, args) = parseIntent(text, pageId: nil) {
                executeBrowserOSCommand(toolName: toolName, args: args, originalText: text)
            } else {
                showUsageHint()
            }
        } else {
            sendToLLM(text)
        }
    }

    private func sendToLLM(_ text: String) {
        isStreaming = true
        queenStatus = .working

        streamingTask?.cancel()
        streamingTask = Task {
            do {
                let history: [LLMClient.Message] = messages.map { msg in
                    let role: String
                    switch msg.role {
                    case .user: role = "user"
                    case .assistant: role = "assistant"
                    case .system: role = "system"
                    case .tool: role = "assistant"
                    }
                    return LLMClient.Message(role: role, content: msg.content)
                }

                let reply = try await llmClient.complete(messages: history)

                let agentMessage = BrowserOSChatMessage(
                    role: .assistant,
                    content: reply,
                    timestamp: Date()
                )
                messages.append(agentMessage)
                queenStatus = .alive

            } catch {
                let errorMessage = BrowserOSChatMessage(
                    role: .system,
                    content: "Agent error: \(error.localizedDescription)",
                    timestamp: Date()
                )
                messages.append(errorMessage)
                queenStatus = .error
            }
            isStreaming = false
        }
    }

    func isLikelyCommand(_ text: String) -> Bool {
        ChatLogic.isLikelyCommand(text)
    }

    private func showUsageHint() {
        isStreaming = true
        let response = """
        BrowserOS Agent ready. Available commands:
        - open [url] - navigate to page
        - click - click element
        - screenshot - capture page
        - extract - get page content
        - shell [command] - run shell command
        """
        let agentMessage = BrowserOSChatMessage(
            role: .assistant,
            content: response,
            timestamp: Date()
        )
        messages.append(agentMessage)
        isStreaming = false
    }

    private func executeBrowserOSCommand(toolName: String, args: [String: Any], originalText: String) {
        isStreaming = true
        queenStatus = .working

        streamingTask?.cancel()
        streamingTask = Task {
            do {
                // Auto-detect page ID before executing browser tools
                let pageId = await ensurePageId()
                var finalArgs = args
                if let pageId = pageId, finalArgs["page"] == nil {
                    finalArgs["page"] = pageId
                }

                let record = ToolCallRecord(
                    name: toolName,
                    status: .running,
                    timestamp: Date(),
                    result: nil
                )
                toolCalls.append(record)

                let response = try await mcpClient.callTool(
                    name: toolName,
                    arguments: finalArgs
                )

                let resultText = extractResultText(response)

                if let index = toolCalls.lastIndex(where: { $0.name == toolName && $0.status == .running }) {
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

    private func parseIntent(_ text: String, pageId: Int?) -> (String, [String: Any])? {
        ChatLogic.parseIntent(text, pageId: pageId)
    }

    private func ensurePageId() async -> Int? {
        if let cached = currentPageId { return cached }
        return await detectPageId()
    }

    private func detectPageId() async -> Int? {
        do {
            // `list_pages` returns a human-readable listing, one page per block:
            //   "0. Title (tab 12)\n   https://example.com"
            // (see apps/server/src/tools/navigation.ts). The previous version
            // JSON-parsed this text, which never matched - page detection always
            // silently failed. Parse the leading page id from the text instead.
            let pagesText = try await mcpClient.listPages()
            if let id = ChatLogic.firstPageId(in: pagesText) {
                currentPageId = id
                return id
            }
            NSLog("[BrowserOSChatViewModel] No page id in list_pages output: \(pagesText.prefix(200))")
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
        "[Q] \(queenStatus.rawValue) \(sessionDuration)"
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