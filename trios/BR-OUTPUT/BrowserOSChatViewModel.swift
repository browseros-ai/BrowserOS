  1 | import Foundation
  2 | import SwiftUI
  3 | import Combine
  4 | 
  5 | @MainActor
  6 | class BrowserOSChatViewModel: ObservableObject {
  7 | 
  8 |     @Published var messages: [BrowserOSChatMessage] = []
  9 |     @Published var isStreaming: Bool = false
 10 |     @Published var isBrowserOSConnected: Bool = false
 11 |     @Published var queenStatus: QueenStatus = .idle
 12 |     @Published var toolCalls: [ToolCallRecord] = []
 13 | 
 14 |     private let mcpClient: TriosMCPClient
 15 |     private var cancellables = Set<AnyCancellable>()
 16 |     private var streamingTask: Task<Void, Never>?
 17 |     private var sessionStartTime: Date = Date()
 18 | 
 19 |     enum QueenStatus: String {
 20 |         case idle = "idle"
 21 |         case alive = "alive"
 22 |         case working = "working"
 23 |         case error = "error"
 24 |     }
 25 | 
 26 |     struct ToolCallRecord: Identifiable {
 27 |         let id = UUID()
 28 |         let name: String
 29 |         let status: ToolStatus
 30 |         let timestamp: Date
 31 |         let result: String?
 32 | 
 33 |         enum ToolStatus {
 34 |             case running, completed, failed
 35 |         }
 36 |     }
 37 | 
 38 |     init() {
 39 |         self.mcpClient = TriosMCPClient()
 40 |         setupHealthCheck()
 41 |     }
 42 | 
 43 |     private func setupHealthCheck() {
 44 |         Task {
 45 |             _ = await mcpClient.checkHealth()
 46 |             Timer.publish(every: 5, on: .main, in: .common)
 47 |                 .autoconnect()
 48 |                 .sink { [weak self] _ in
 49 |                     Task {
 50 |                         await self?.updateConnectionStatus()
 51 |                     }
 52 |                 }
 53 |                 .store(in: &cancellables)
 54 |         }
 55 |     }
 56 | 
 57 |     private func updateConnectionStatus() async {
 58 |         let connected = await mcpClient.checkHealth()
 59 |         self.isBrowserOSConnected = connected
 60 |         if connected && queenStatus == .idle {
 61 |             self.queenStatus = .alive
 62 |         } else if !connected {
 63 |             self.queenStatus = .error
 64 |         }
 65 |     }
 66 | 
 67 |     func sendMessage(_ text: String) {
 68 |         let userMessage = BrowserOSChatMessage(role: .user, content: text, timestamp: Date())
 69 |         messages.append(userMessage)
 70 | 
 71 |         if shouldUseBrowserOS(text) {
 72 |             executeBrowserOSCommand(text)
 73 |         } else {
 74 |             streamLocalResponse(text)
 75 |         }
 76 |     }
 77 | 
 78 |     private func executeBrowserOSCommand(_ text: String) {
 79 |         isStreaming = true
 80 |         queenStatus = .working
 81 | 
 82 |         streamingTask = Task {
 83 |             do {
 84 |                 let (toolName, args) = parseIntent(text)
 85 | 
 86 |                 let record = ToolCallRecord(
 87 |                     name: toolName,
 88 |                     status: .running,
 89 |                     timestamp: Date(),
 90 |                     result: nil
 91 |                 )
 92 |                 toolCalls.append(record)
 93 | 
 94 |                 let response = try await mcpClient.callTool(
 95 |                     name: toolName,
 96 |                     arguments: args
 97 |                 )
 98 | 
 99 |                 let resultText = extractResultText(response)
100 | 
101 |                 if let index = toolCalls.lastIndex(where: { $0.name == toolName }) {
102 |                     toolCalls[index] = ToolCallRecord(
103 |                         name: toolName,
104 |                         status: .completed,
105 |                         timestamp: toolCalls[index].timestamp,
106 |                         result: resultText
107 |                     )
108 |                 }
109 | 
110 |                 let agentMessage = BrowserOSChatMessage(
111 |                     role: .assistant,
112 |                     content: resultText,
113 |                     timestamp: Date(),
114 |                     toolCalls: [BrowserOSToolCall(name: toolName, result: resultText)]
115 |                 )
116 |                 messages.append(agentMessage)
117 | 
118 |                 queenStatus = .alive
119 | 
120 |             } catch {
121 |                 let errorMessage = BrowserOSChatMessage(
122 |                     role: .system,
123 |                     content: "BrowserOS Error: \(error.localizedDescription)",
124 |                     timestamp: Date()
125 |                 )
126 |                 messages.append(errorMessage)
127 |                 queenStatus = .error
128 |             }
129 | 
130 |             isStreaming = false
131 |         }
132 |     }
133 | 
134 |     private func parseIntent(_ text: String) -> (String, [String: Any]) {
135 |         let lower = text.lowercased()
136 | 
137 |         if lower.contains("navigate") || lower.contains("go to") || lower.contains("open") {
138 |             let url = extractURL(from: text) ?? "https://google.com"
139 |             return ("navigate_page", ["page": 1, "url": url])
140 |         }
141 | 
142 |         if lower.contains("click") || lower.contains("press") {
143 |             return ("click", ["page": 1, "element": 1])
144 |         }
145 | 
146 |         if lower.contains("screenshot") || lower.contains("capture") {
147 |             return ("take_screenshot", ["page": 1])
148 |         }
149 | 
150 |         if lower.contains("extract") || lower.contains("get data") {
151 |             return ("get_page_content", ["page": 1])
152 |         }
153 | 
154 |         return ("filesystem_bash", ["command": text, "description": "User command"])
155 |     }
156 | 
157 |     private func extractURL(from text: String) -> String? {
158 |         let pattern = #"(https?://[^\s]+)"#
159 |         guard let regex = try? NSRegularExpression(pattern: pattern) else { return nil }
160 |         let range = NSRange(text.startIndex..., in: text)
161 |         if let match = regex.firstMatch(in: text, range: range) {
162 |             return String(text[Range(match.range, in: text)!])
163 |         }
164 |         return nil
165 |     }
166 | 
167 |     private func shouldUseBrowserOS(_ text: String) -> Bool {
168 |         let commands = ["navigate", "click", "screenshot", "extract", "open", "go to", "browse"]
169 |         return commands.contains { text.lowercased().contains($0) }
170 |     }
171 | 
172 |     private func streamLocalResponse(_ text: String) {
173 |         isStreaming = true
174 | 
175 |         let response = "BrowserOS bridge active. Try: open google.com, screenshot, extract data"
176 | 
177 |         let agentMessage = BrowserOSChatMessage(
178 |             role: .assistant,
179 |             content: response,
180 |             timestamp: Date()
181 |         )
182 |         messages.append(agentMessage)
183 |         isStreaming = false
184 |     }
185 | 
186 |     private func extractResultText(_ response: MCPResponse) -> String {
187 |         guard let result = response.result else { return "No result" }
188 |         return result.content.compactMap { $0.text }.joined(separator: "\n")
189 |     }
190 | 
191 |     var sessionDuration: String {
192 |         let interval = Date().timeIntervalSince(sessionStartTime)
193 |         let minutes = Int(interval) / 60
194 |         return minutes > 0 ? "\(minutes)m" : "\(Int(interval))s"
195 |     }
196 | 
197 |     var queenStatusText: String {
198 |         "👑 \(queenStatus.rawValue) \(sessionDuration)"
199 |     }
200 | }
201 | 
202 | struct BrowserOSChatMessage: Identifiable {
203 |     let id = UUID()
204 |     let role: ChatRole
205 |     let content: String
206 |     let timestamp: Date
207 |     var toolCalls: [BrowserOSToolCall] = []
208 | 
209 |     enum ChatRole {
210 |         case user, assistant, system, tool
211 |     }
212 | }
213 | 
214 | struct BrowserOSToolCall {
215 |     let name: String
216 |     let result: String?
217 | }
218 | 