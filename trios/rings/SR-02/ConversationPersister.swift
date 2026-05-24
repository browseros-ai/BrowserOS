import Foundation

actor ConversationPersister: ChatPersisterProtocol {
    private let defaults = UserDefaults.standard
    private let keyPrefix = "trios.conversation."
    private let currentIdKey = "trios.currentConversationId"

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

    nonisolated func currentConversationId() -> UUID {
        guard let str = UserDefaults.standard.string(forKey: currentIdKey),
              let id = UUID(uuidString: str) else {
            let newId = UUID()
            UserDefaults.standard.set(newId.uuidString, forKey: currentIdKey)
            return newId
        }
        return id
    }

    nonisolated func setCurrentConversationId(_ id: UUID) {
        UserDefaults.standard.set(id.uuidString, forKey: currentIdKey)
    }

    func listAllConversations() async -> [ChatConversation] {
        var result: [ChatConversation] = []
        for key in defaults.dictionaryRepresentation().keys {
            guard key.hasPrefix(keyPrefix) else { continue }
            let idStr = String(key.dropFirst(keyPrefix.count))
            guard let id = UUID(uuidString: idStr) else { continue }
            let messages = await load(conversationId: id)
            let title = messages.first(where: { $0.role == .user })?.content.prefix(40).trimmingCharacters(in: .whitespacesAndNewlines) ?? "Empty chat"
            let updated = messages.last?.timestamp ?? Date()
            result.append(ChatConversation(id: id, title: String(title), updatedAt: updated))
        }
        return result.sorted { $0.updatedAt > $1.updatedAt }
    }
}
