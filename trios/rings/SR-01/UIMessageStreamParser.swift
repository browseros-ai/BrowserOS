import Foundation

actor UIMessageStreamParser: ChatParserProtocol {
    private var currentMessageId: UUID?
    private var currentToolCallId: String?

    func parse(_ event: SSEEvent) async -> ParserAction? {
        switch event {
        case .start(_):
            let messageId = UUID()
            currentMessageId = messageId
            return .appendMessage(ChatMessage(id: messageId, role: .assistant, isStreaming: true))

        case .textStart:
            return nil

        case .textDelta(_, let delta):
            guard let messageId = currentMessageId else { return nil }
            return .appendText(messageId: messageId, delta: delta)

        case .textEnd:
            guard let messageId = currentMessageId else { return nil }
            return .finishMessage(messageId: messageId)

        case .reasoningStart:
            guard let messageId = currentMessageId else { return nil }
            return .startSegment(messageId: messageId, segment: .reasoning(""))

        case .reasoningDelta(_, let delta):
            guard let messageId = currentMessageId else { return nil }
            return .appendToSegment(messageId: messageId, kind: .reasoning, delta: delta)

        case .reasoningEnd:
            return nil

        case .toolInputStart(_, let toolCallId, let name):
            currentToolCallId = toolCallId
            guard let messageId = currentMessageId else { return nil }
            let toolCall = ToolCall(id: toolCallId, name: name, arguments: "", output: nil, isComplete: false)
            return .addToolCall(messageId: messageId, toolCall: toolCall)

        case .toolInputDelta(_, let delta):
            guard let messageId = currentMessageId,
                  let toolCallId = currentToolCallId else { return nil }
            return .appendToolInput(messageId: messageId, toolCallId: toolCallId, delta: delta)

        case .toolInputAvailable(_, let toolCallId, let argsData):
            guard let messageId = currentMessageId else { return nil }
            let args = String(data: argsData, encoding: .utf8) ?? ""
            return .finalizeToolInput(messageId: messageId, toolCallId: toolCallId, arguments: args)

        case .toolOutputAvailable(_, let toolCallId, let resultData):
            guard let messageId = currentMessageId else { return nil }
            let result = String(data: resultData, encoding: .utf8) ?? ""
            return .setToolOutput(messageId: messageId, toolCallId: toolCallId, output: result)

        case .toolOutputError(_, let toolCallId, let error):
            guard let messageId = currentMessageId else { return nil }
            return .setToolError(messageId: messageId, toolCallId: toolCallId, error: error)

        case .finish:
            currentMessageId = nil
            currentToolCallId = nil
            return .streamComplete

        case .abort:
            currentMessageId = nil
            currentToolCallId = nil
            return .streamAborted

        case .error(_, let message):
            currentMessageId = nil
            currentToolCallId = nil
            return .streamError(message)

        case .ping, .unknown:
            return nil
        }
    }

    func reset() async {
        currentMessageId = nil
        currentToolCallId = nil
    }
}
