import Foundation

final class ChatMessage: Identifiable, Codable, Equatable {
    let id: UUID
    var role: ChatRole
    var content: String
    var segments: [MessageSegment]
    var timestamp: Date
    var isStreaming: Bool
    var toolCalls: [ToolCall]
    var task: AgentTask?

    init(
        id: UUID = UUID(),
        role: ChatRole,
        content: String = "",
        segments: [MessageSegment] = [],
        timestamp: Date = Date(),
        isStreaming: Bool = false,
        toolCalls: [ToolCall] = [],
        task: AgentTask? = nil
    ) {
        self.id = id
        self.role = role
        self.content = content
        self.segments = segments
        self.timestamp = timestamp
        self.isStreaming = isStreaming
        self.toolCalls = toolCalls
        self.task = task
    }

    static func == (lhs: ChatMessage, rhs: ChatMessage) -> Bool {
        lhs.id == rhs.id &&
        lhs.role == rhs.role &&
        lhs.content == rhs.content &&
        lhs.segments == rhs.segments &&
        lhs.timestamp == rhs.timestamp &&
        lhs.isStreaming == rhs.isStreaming &&
        lhs.toolCalls == rhs.toolCalls &&
        lhs.task == rhs.task
    }
}

enum ChatRole: String, Codable, Equatable {
    case user
    case assistant
    case system
    case tool
}

enum MessageSegment: Codable, Equatable, Hashable {
    case text(String)
    case reasoning(String)
    case toolInput(name: String, arguments: String)
    case toolOutput(name: String, result: String)
    case error(String)

    enum CodingKeys: String, CodingKey {
        case kind, text, name, arguments, result
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let kind = try container.decode(String.self, forKey: .kind)
        switch kind {
        case "text":
            self = .text(try container.decode(String.self, forKey: .text))
        case "reasoning":
            self = .reasoning(try container.decode(String.self, forKey: .text))
        case "toolInput":
            self = .toolInput(
                name: try container.decode(String.self, forKey: .name),
                arguments: try container.decode(String.self, forKey: .arguments)
            )
        case "toolOutput":
            self = .toolOutput(
                name: try container.decode(String.self, forKey: .name),
                result: try container.decode(String.self, forKey: .result)
            )
        case "error":
            self = .error(try container.decode(String.self, forKey: .text))
        default:
            throw DecodingError.dataCorruptedError(forKey: .kind, in: container, debugDescription: "Unknown segment kind")
        }
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .text(let text):
            try container.encode("text", forKey: .kind)
            try container.encode(text, forKey: .text)
        case .reasoning(let text):
            try container.encode("reasoning", forKey: .kind)
            try container.encode(text, forKey: .text)
        case .toolInput(let name, let arguments):
            try container.encode("toolInput", forKey: .kind)
            try container.encode(name, forKey: .name)
            try container.encode(arguments, forKey: .arguments)
        case .toolOutput(let name, let result):
            try container.encode("toolOutput", forKey: .kind)
            try container.encode(name, forKey: .name)
            try container.encode(result, forKey: .result)
        case .error(let text):
            try container.encode("error", forKey: .kind)
            try container.encode(text, forKey: .text)
        }
    }
}

struct ToolCall: Codable, Equatable, Identifiable {
    let id: String
    var name: String
    var arguments: String
    var output: String?
    var isComplete: Bool
}
