import Foundation
import Combine

// MARK: - SessionGuard
/// Actor that keeps BrowserOS session alive via proactive health checks
/// Implements Trinity session-longevity skill
actor SessionGuard {
    
    // MARK: Properties
    private var lastActivity: Date
    private let timeout: TimeInterval = 300 // 5 min warning
    private let heartbeatInterval: TimeInterval = 120 // 2 min heartbeat
    private var heartbeatTask: Task<Void, Never>?
    private var mcpClient: TriosMCPClient?
    
    // MARK: Published (via continuation)
    private var statusContinuation: AsyncStream<SessionStatus>.Continuation?
    let statusStream: AsyncStream<SessionStatus>
    
    enum SessionStatus {
        case healthy(elapsed: TimeInterval)
        case warning(elapsed: TimeInterval)
        case expired
        case pingSuccess
        case pingFailure(Error)
    }
    
    // MARK: Init
    init() {
        self.lastActivity = Date()
        var continuation: AsyncStream<SessionStatus>.Continuation!
        self.statusStream = AsyncStream { cont in
            continuation = cont
        }
        self.statusContinuation = continuation
    }
    
    // MARK: - Activity Tracking
    func ping() {
        lastActivity = Date()
    }
    
    func checkVitality() -> Bool {
        let elapsed = Date().timeIntervalSince(lastActivity)
        return elapsed < timeout
    }
    
    // MARK: - Heartbeat Loop
    func startHeartbeat(mcpClient: TriosMCPClient) {
        self.mcpClient = mcpClient
        heartbeatTask?.cancel()
        
        heartbeatTask = Task {
            while !Task.isCancelled {
                do {
                    let connected = await mcpClient.checkHealth()
                    if connected {
                        ping()
                        statusContinuation?.yield(.pingSuccess)
                    }
                } catch {
                    statusContinuation?.yield(.pingFailure(error))
                }
                
                // Check session age
                let elapsed = Date().timeIntervalSince(lastActivity)
                if elapsed > timeout {
                    statusContinuation?.yield(.expired)
                } else if elapsed > timeout * 0.8 {
                    statusContinuation?.yield(.warning(elapsed: elapsed))
                } else {
                    statusContinuation?.yield(.healthy(elapsed: elapsed))
                }
                
                try? await Task.sleep(nanoseconds: UInt64(heartbeatInterval * 1_000_000_000))
            }
        }
    }
    
    // MARK: - Auto-Save
    func autoSaveState(messages: [BrowserOSChatMessage], to path: String) async {
        do {
            let data = try JSONEncoder().encode(messages)
            let url = URL(fileURLWithPath: path)
            try data.write(to: url)
            ping()
        } catch {
            print("Auto-save failed: (error)")
        }
    }
    
    // MARK: - Session Recovery
    func recoverState(from path: String) async -> [BrowserOSChatMessage]? {
        do {
            let url = URL(fileURLWithPath: path)
            let data = try Data(contentsOf: url)
            let messages = try JSONDecoder().decode([BrowserOSChatMessage].self, from: data)
            ping()
            return messages
        } catch {
            return nil
        }
    }
    
    // MARK: - Deinit
    deinit {
        heartbeatTask?.cancel()
        statusContinuation?.finish()
    }
}

// MARK: - Codable Support
extension BrowserOSChatMessage: Codable {
    enum CodingKeys: String, CodingKey {
        case id, role, content, timestamp, toolCalls
    }
    
    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(id, forKey: .id)
        try container.encode(role.rawValue, forKey: .role)
        try container.encode(content, forKey: .content)
        try container.encode(timestamp, forKey: .timestamp)
        try container.encode(toolCalls, forKey: .toolCalls)
    }
}

extension BrowserOSChatMessage.ChatRole: RawRepresentable {
    var rawValue: String {
        switch self {
        case .user: return "user"
        case .assistant: return "assistant"
        case .system: return "system"
        case .tool: return "tool"
        }
    }
    
    init?(rawValue: String) {
        switch rawValue {
        case "user": self = .user
        case "assistant": self = .assistant
        case "system": self = .system
        case "tool": self = .tool
        default: return nil
        }
    }
}