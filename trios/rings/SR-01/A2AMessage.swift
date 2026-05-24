import Foundation

struct A2AMessage: Codable, Identifiable, Sendable {
    let id: UUID
    let sender: AgentId
    let recipient: AgentId?
    let type: A2AMessageType
    let payload: Data
    let timestamp: String

    init(
        id: UUID = UUID(),
        sender: AgentId,
        recipient: AgentId? = nil,
        type: A2AMessageType,
        payload: Data,
        timestamp: String = ISO8601DateFormatter().string(from: Date())
    ) {
        self.id = id
        self.sender = sender
        self.recipient = recipient
        self.type = type
        self.payload = payload
        self.timestamp = timestamp
    }
}

enum A2AMessageType: String, Codable, Sendable {
    case direct, broadcast, taskAssign, taskUpdate, taskResult, heartbeat, error
}

struct AgentTask: Codable, Identifiable, Sendable, Equatable {
    let id: UUID
    let title: String
    let description: String
    var state: AgentTaskState
    let priority: AgentTaskPriority
    let assignee: AgentId
    let createdAt: String
    var updatedAt: String
}

enum AgentTaskState: String, Codable, Sendable {
    case pending, assigned, inProgress, completed, failed, cancelled
}

enum AgentTaskPriority: Int, Codable, Sendable, Comparable {
    case low = 0, medium = 1, high = 2, critical = 3

    static func < (lhs: AgentTaskPriority, rhs: AgentTaskPriority) -> Bool {
        lhs.rawValue < rhs.rawValue
    }
}
