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
    
    private let mcpClient: TriosMCPClient
    private var cancellables = Set<AnyCancellable>()
    private var streamingTask: Task<Void, Never>?
    private var sessionStartTime: Date = Date()
    
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
        
        if shouldUseBrowserOS(text) {
            executeBrowserOSCommand(text)
        } else {
            streamLocalResponse(text)
        }
    }
    
    private func executeBrowserOSCommand(_ text: String) {
        isStreaming = true
        queenStatus = .working
        
        streamingTask = Task {
            do {
                let (toolName, args) = parseIntent(text)
                
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
                    content: "BrowserOS Error: (error.localizedDescription)",
                    timestamp: Date()
                )
                messages.append(errorMessage)
                queenStatus = .error
            }
            
            isStreaming = false
        }
    }
    
    private func parseIntent(_ text: String) -> (String, [String: Any]) {
        let lower = text.lowercased()
        
        if lower.contains("navigate") || lower.contains("go to") || lower.contains("open") {
            let url = extractURL(from: text) ?? "https://google.com"
            return ("shell_execute", ["command": "open (url)", "description": "Open URL"])
        }
        
        if lower.contains("click") || lower.contains("press") {
            return ("shell_execute", ["command": "echo Click command received", "description": "Click"])
        }
        
        if lower.contains("screenshot") || lower.contains("capture") {
            return ("take_screenshot", ["page": 1])
        }
        
        if lower.contains("extract") || lower.contains("get data") {
            return ("get_page_content", ["page": 1])
        }
        
        return ("shell_execute", ["command": text, "description": "User command"])
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
    
    private func shouldUseBrowserOS(_ text: String) -> Bool {
        let commands = ["navigate", "click", "screenshot", "extract", "open", "go to", "browse"]
        return commands.contains { text.lowercased().contains($0) }
    }
    
    private func streamLocalResponse(_ text: String) {
        isStreaming = true
        let response = "BrowserOS bridge active. Try: open google.com, screenshot, extract data"
        let agentMessage = BrowserOSChatMessage(
            role: .assistant,
            content: response,
            timestamp: Date()
        )
        messages.append(agentMessage)
        isStreaming = false
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