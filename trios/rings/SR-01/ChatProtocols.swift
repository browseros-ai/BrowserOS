import Foundation

protocol ChatTransportProtocol: Sendable {
    func sendMessage(body: Data) async throws -> AsyncStream<SSEEvent>
    func cancel() async
}

protocol ChatParserProtocol: Sendable {
    func parse(_ event: SSEEvent) async -> ParserAction?
    func reset() async
}

struct ChatConversation: Identifiable, Codable, Equatable {
    let id: UUID
    let title: String
    let updatedAt: Date
}

protocol ChatPersisterProtocol: Sendable {
    func save(messages: [ChatMessage], conversationId: UUID) async
    func load(conversationId: UUID) async -> [ChatMessage]
    func clear(conversationId: UUID) async
    func currentConversationId() -> UUID
    func setCurrentConversationId(_ id: UUID)
    func listAllConversations() async -> [ChatConversation]
}

protocol ChatHealthCheckProtocol: Sendable {
    func check() async -> Bool
}
