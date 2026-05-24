import Foundation
import SwiftUI

@MainActor
final class A2AMessageRouter {
    private weak var viewModel: ChatViewModel?

    init(viewModel: ChatViewModel) {
        self.viewModel = viewModel
    }

    func route(_ message: A2AMessage) {
        switch message.type {
        case .direct, .broadcast:
            handleChatMessage(message)
        case .taskAssign:
            handleAgentTaskAssign(message)
        case .taskUpdate:
            handleAgentTaskUpdate(message)
        case .heartbeat:
            break
        case .taskResult:
            handleAgentTaskResult(message)
        case .error:
            handleError(message)
        }
    }

    private func handleChatMessage(_ message: A2AMessage) {
        guard let text = String(data: message.payload, encoding: .utf8) else { return }
        let chatMessage = ChatMessage(
            role: .assistant,
            content: text,
            segments: [.text(text)]
        )
        viewModel?.messages.append(chatMessage)
        viewModel?.rebuildCache()
        viewModel?.objectWillChange.send()
    }

    private func handleAgentTaskAssign(_ message: A2AMessage) {
        guard let task = try? JSONDecoder().decode(AgentTask.self, from: message.payload) else { return }
        let chatMessage = ChatMessage(
            role: .assistant,
            content: "Task assigned: \(task.title)",
            segments: [.text("Task assigned: \(task.title)")],
            task: task
        )
        viewModel?.messages.append(chatMessage)
        viewModel?.rebuildCache()
        viewModel?.objectWillChange.send()
    }

    private func handleAgentTaskUpdate(_ message: A2AMessage) {
        guard let updatedTask = try? JSONDecoder().decode(AgentTask.self, from: message.payload) else { return }
        guard let index = viewModel?.messages.firstIndex(where: { $0.task?.id == updatedTask.id }) else { return }
        viewModel?.messages[index].task = updatedTask
        viewModel?.objectWillChange.send()
    }

    private func handleAgentTaskResult(_ message: A2AMessage) {
        guard let task = try? JSONDecoder().decode(AgentTask.self, from: message.payload) else { return }
        guard let index = viewModel?.messages.firstIndex(where: { $0.task?.id == task.id }) else { return }
        viewModel?.messages[index].task = task
        viewModel?.objectWillChange.send()
    }

    private func handleError(_ message: A2AMessage) {
        guard let text = String(data: message.payload, encoding: .utf8) else { return }
        let chatMessage = ChatMessage(
            role: .system,
            content: "",
            segments: [.error(text)]
        )
        viewModel?.messages.append(chatMessage)
        viewModel?.rebuildCache()
        viewModel?.objectWillChange.send()
    }
}
