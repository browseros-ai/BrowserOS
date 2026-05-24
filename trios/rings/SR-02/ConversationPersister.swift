import Foundation

actor ConversationPersister: ChatPersisterProtocol {
    private let defaults = UserDefaults.standard
    private let keyPrefix = "trios.conversation."

    func save(messages: [ChatMessage], conversationId: UUID) async {
        let key = keyPrefix + conversationId.uuidString
        if let data = try? JSONEncoder().encode(messages) {
            defaults.set(data, forKey: key)
        }
    }

    func load(conversationId: UUID) async -> [ChatMessage] {
        let key = keyPrefix + conversationId.uuidString
        guard let data = defaults.data(forKey: key),
              let messages = try? JSONDecoder().decode([ChatMessage].self, from: data) else {
            return []
        }
        return messages
    }

    func clear(conversationId: UUID) async {
        let key = keyPrefix + conversationId.uuidString
        defaults.removeObject(forKey: key)
    }
}
