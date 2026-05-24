import Foundation

protocol ChatTransportProtocol: Sendable {
    func sendMessage(body: Data) async throws -> AsyncStream<SSEEvent>
    func cancel() async
}

protocol ChatParserProtocol: Sendable {
    func parse(_ event: SSEEvent) async -> ParserAction?
    func reset() async
}

protocol ChatPersisterProtocol: Sendable {
    func save(messages: [ChatMessage], conversationId: UUID) async
    func load(conversationId: UUID) async -> [ChatMessage]
    func clear(conversationId: UUID) async
}

protocol ChatHealthCheckProtocol: Sendable {
    func check() async -> Bool
}
