import Foundation

struct AgentId: RawRepresentable, Hashable, Codable, Sendable {
    let rawValue: String
    init(_ rawValue: String) { self.rawValue = rawValue }
    init(rawValue: String) { self.rawValue = rawValue }
}

struct AgentCard: Codable, Sendable {
    let id: AgentId
    let name: String
    let description: String
    let capabilities: [Capability]
    let version: String
    let endpoint: URL?
}

enum Capability: String, Codable, Sendable, CaseIterable {
    case browserControl, chat, git, shell, fileSystem, orchestrator
}

enum AgentStatus: String, Codable, Sendable {
    case idle, busy, offline, error
}
