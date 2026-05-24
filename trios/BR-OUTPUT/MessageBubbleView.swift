import SwiftUI

struct MessageBubbleView: View {
    let message: ChatMessage
    var onTaskAction: ((UUID, AgentTaskState) -> Void)?
    var onRegenerate: (() -> Void)?
    var onFeedback: ((Bool) -> Void)?

    var body: some View {
        HStack {
            if message.role == .user {
                Spacer(minLength: 40)
            }

            if message.role == .assistant {
                assistantContainer
            } else {
                userContent
            }

            if message.role == .assistant {
                Spacer(minLength: 40)
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
    }

    // MARK: - User Message

    private var userContent: some View {
        Group {
            if !message.content.isEmpty {
                RichMessageView(text: message.content, isUser: true)
                    .foregroundColor(.primary)
                    .textSelection(.enabled)
            }

            if message.isStreaming && message.content.isEmpty {
                TypingIndicatorView()
                    .foregroundColor(.primary)
            }
        }
    }

    // MARK: - Assistant Container

    private var assistantContainer: some View {
        VStack(alignment: .leading, spacing: 16) {
            // Reasoning (collapsible, hidden by default)
            ForEach(reasoningSegments, id: \.self) { text in
                ReasoningCollapsibleView(content: text)
            }

            // Tool calls
            ForEach(Array(message.toolCalls.enumerated()), id: \.element.id) { index, toolCall in
                ToolCallCardView(toolCall: toolCall)
            }

            // Task
            if let task = message.task {
                AgentTaskBubbleView(
                    task: task,
                    onAccept: { onTaskAction?(task.id, .assigned) },
                    onReject: { onTaskAction?(task.id, .cancelled) },
                    onComplete: { onTaskAction?(task.id, .completed) }
                )
            }

            // Main content
            if !message.content.isEmpty {
                RichMessageView(text: message.content, isUser: false)
                    .textSelection(.enabled)
            }

            // Streaming indicator
            if message.isStreaming && message.content.isEmpty && message.segments.isEmpty {
                TypingIndicatorView()
            }

            // Action bar
            if !message.isStreaming && !message.content.isEmpty {
                AssistantActionBar(
                    content: message.content,
                    onRegenerate: onRegenerate,
                    onFeedback: onFeedback
                )
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
        .background(Color(NSColor.controlBackgroundColor))
        .cornerRadius(8)
    }

    private var reasoningSegments: [String] {
        message.segments.compactMap {
            if case .reasoning(let text) = $0 { return text }
            return nil
        }
    }
}

// MARK: - Assistant Action Bar

private struct AssistantActionBar: View {
    let content: String
    var onRegenerate: (() -> Void)?
    var onFeedback: ((Bool) -> Void)?

    @State private var copied = false
    @State private var liked: Bool? = nil

    var body: some View {
        HStack(spacing: 14) {
            Button(action: {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(content, forType: .string)
                copied = true
                DispatchQueue.main.asyncAfter(deadline: .now() + 2) {
                    copied = false
                }
            }) {
                Image(systemName: copied ? "checkmark" : "doc.on.doc")
                    .font(.system(size: 13))
                    .foregroundColor(copied ? .primary : .gray)
            }
            .buttonStyle(PlainButtonStyle())
            .help("Copy")

            Button(action: {
                onRegenerate?()
            }) {
                Image(systemName: "arrow.clockwise")
                    .font(.system(size: 13))
                    .foregroundColor(.gray)
            }
            .buttonStyle(PlainButtonStyle())
            .help("Regenerate")

            Spacer()

            Button(action: {
                liked = liked == true ? nil : true
                onFeedback?(true)
            }) {
                Image(systemName: liked == true ? "hand.thumbsup.fill" : "hand.thumbsup")
                    .font(.system(size: 13))
                    .foregroundColor(liked == true ? .primary : .gray)
            }
            .buttonStyle(PlainButtonStyle())
            .help("Good response")

            Button(action: {
                liked = liked == false ? nil : false
                onFeedback?(false)
            }) {
                Image(systemName: liked == false ? "hand.thumbsdown.fill" : "hand.thumbsdown")
                    .font(.system(size: 13))
                    .foregroundColor(liked == false ? .primary : .gray)
            }
            .buttonStyle(PlainButtonStyle())
            .help("Bad response")
        }
        .padding(.top, 4)
    }
}
