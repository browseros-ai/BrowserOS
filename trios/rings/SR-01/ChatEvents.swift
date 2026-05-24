import Foundation

enum ConversationState: Equatable {
    case idle
    case streaming(messageId: UUID)
    case error(String)
    case reconnecting(attempt: Int, maxAttempts: Int)
}

enum SSEEvent: Equatable {
    case start(id: String)
    case textStart(id: String)
    case textDelta(id: String, delta: String)
    case textEnd(id: String)
    case reasoningStart(id: String)
    case reasoningDelta(id: String, delta: String)
    case reasoningEnd(id: String)
    case toolInputStart(id: String, toolCallId: String, name: String)
    case toolInputDelta(id: String, delta: String)
    case toolInputAvailable(id: String, toolCallId: String, args: Data)
    case toolOutputAvailable(id: String, toolCallId: String, result: Data)
    case toolOutputError(id: String, toolCallId: String, error: String)
    case finish(id: String)
    case abort(id: String)
    case error(id: String, message: String)
    case ping
    case unknown(data: String)
}

enum ParserAction: Equatable {
    case appendMessage(ChatMessage)
    case appendText(messageId: UUID, delta: String)
    case finishMessage(messageId: UUID)
    case startSegment(messageId: UUID, segment: MessageSegment)
    case appendToSegment(messageId: UUID, kind: SegmentKind, delta: String)
    case addToolCall(messageId: UUID, toolCall: ToolCall)
    case appendToolInput(messageId: UUID, toolCallId: String, delta: String)
    case finalizeToolInput(messageId: UUID, toolCallId: String, arguments: String)
    case setToolOutput(messageId: UUID, toolCallId: String, output: String)
    case setToolError(messageId: UUID, toolCallId: String, error: String)
    case streamComplete
    case streamAborted
    case streamError(String)
}

enum SegmentKind: Equatable {
    case text
    case reasoning
}

struct SSEEventParser {
    static func parse(line: String) -> SSEEvent? {
        guard line.hasPrefix("data: ") else { return nil }
        let json = String(line.dropFirst(6))
        guard let data = json.data(using: .utf8),
              let dict = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let type = dict["type"] as? String else {
            return .unknown(data: json)
        }

        let id = dict["id"] as? String ?? dict["messageId"] as? String ?? UUID().uuidString

        switch type {
        case "start":
            return .start(id: id)
        case "text-start":
            return .textStart(id: id)
        case "text-delta":
            let delta = dict["delta"] as? String ?? ""
            return .textDelta(id: id, delta: delta)
        case "text-end":
            return .textEnd(id: id)
        case "reasoning-start":
            return .reasoningStart(id: id)
        case "reasoning-delta":
            let delta = dict["delta"] as? String ?? ""
            return .reasoningDelta(id: id, delta: delta)
        case "reasoning-end":
            return .reasoningEnd(id: id)
        case "tool-input-start":
            let toolCallId = dict["toolCallId"] as? String ?? ""
            let name = dict["toolName"] as? String ?? dict["name"] as? String ?? ""
            return .toolInputStart(id: id, toolCallId: toolCallId, name: name)
        case "tool-input-delta":
            let delta = dict["inputTextDelta"] as? String ?? dict["delta"] as? String ?? ""
            return .toolInputDelta(id: id, delta: delta)
        case "tool-input-available":
            let toolCallId = dict["toolCallId"] as? String ?? ""
            let input = dict["input"]
            let argsString: String
            if let str = input as? String {
                argsString = str
            } else if let obj = input,
                      let data = try? JSONSerialization.data(withJSONObject: obj, options: .prettyPrinted) {
                argsString = String(data: data, encoding: .utf8) ?? ""
            } else {
                argsString = ""
            }
            let argsData = argsString.data(using: .utf8) ?? Data()
            return .toolInputAvailable(id: id, toolCallId: toolCallId, args: argsData)
        case "tool-output-available":
            let toolCallId = dict["toolCallId"] as? String ?? ""
            let output = dict["output"]
            let resultString: String
            if let str = output as? String {
                resultString = str
            } else if let obj = output,
                      let data = try? JSONSerialization.data(withJSONObject: obj, options: .prettyPrinted) {
                resultString = String(data: data, encoding: .utf8) ?? ""
            } else {
                resultString = ""
            }
            let resultData = resultString.data(using: .utf8) ?? Data()
            return .toolOutputAvailable(id: id, toolCallId: toolCallId, result: resultData)
        case "tool-output-error":
            let toolCallId = dict["toolCallId"] as? String ?? ""
            let error = dict["errorText"] as? String ?? dict["error"] as? String ?? "Unknown error"
            return .toolOutputError(id: id, toolCallId: toolCallId, error: error)
        case "finish":
            return .finish(id: id)
        case "abort":
            return .abort(id: id)
        case "error":
            let message = dict["errorText"] as? String ?? dict["message"] as? String ?? "Unknown error"
            return .error(id: id, message: message)
        default:
            return .unknown(data: json)
        }
    }
}
