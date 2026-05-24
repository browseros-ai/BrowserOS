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
                userBubble
            }

            if message.role == .assistant {
                Spacer(minLength: 40)
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
    }

    // MARK: - User Message

    private var userBubble: some View {
        Group {
            if !message.content.isEmpty {
                RichMessageView(text: message.content, isUser: true)
                    .font(.system(size: 15, weight: .regular, design: .default))
                    .foregroundColor(.grokText)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 10)
                    .textSelection(.enabled)
            }

            if message.isStreaming && message.content.isEmpty {
                TypingIndicatorView()
                    .foregroundColor(.grokText)
            }
        }
    }

    // MARK: - Assistant Container

    private var assistantContainer: some View {
        VStack(alignment: .leading, spacing: 14) {
            // Reasoning header
            if !reasoningSegments.isEmpty {
                HStack(spacing: 6) {
                    Image(systemName: "clock")
                        .font(.system(size: 11))
                        .foregroundColor(.grokMuted)
                    Text("Thought for \(reasoningDuration)")
                        .font(.system(size: 12, weight: .medium, design: .default))
                        .foregroundColor(.grokMuted)
                }
            }

            // Reasoning (collapsible)
            ForEach(reasoningSegments, id: \.self) { text in
                ReasoningCollapsibleView(content: text)
            }

            // Main content
            if !message.content.isEmpty {
                RichMessageView(text: message.content, isUser: false)
                    .font(.system(size: 15, weight: .regular, design: .default))
                    .foregroundColor(.grokText)
                    .textSelection(.enabled)
            }

            // Streaming indicator
            if message.isStreaming && message.content.isEmpty && message.segments.isEmpty {
                TypingIndicatorView()
            }

            // Inline action bar
            if !message.isStreaming && !message.content.isEmpty {
                MessageActionBar(
                    content: message.content,
                    onRegenerate: onRegenerate,
                    onFeedback: onFeedback
                )
            }
        }
    }

    private var reasoningSegments: [String] {
        message.segments.compactMap {
            if case .reasoning(let text) = $0 { return text }
            return nil
        }
    }

    private var reasoningDuration: String {
        // Approximate: 1 second per line of reasoning
        let lines = reasoningSegments.reduce(0) { count, text in
            count + text.components(separatedBy: .newlines).filter { !$0.trimmingCharacters(in: .whitespaces).isEmpty }.count
        }
        return lines <= 1 ? "1s" : "\(lines)s"
    }
}

// MARK: - Message Action Bar

private struct MessageActionBar: View {
    let content: String
    var onRegenerate: (() -> Void)?
    var onFeedback: ((Bool) -> Void)?

    @State private var copied = false
    @State private var liked: Bool? = nil

    var body: some View {
        HStack(spacing: 18) {
            Button(action: {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(content, forType: .string)
                copied = true
                DispatchQueue.main.asyncAfter(deadline: .now() + 2) {
                    copied = false
                }
            }) {
                Image(systemName: copied ? "checkmark" : "doc.on.doc")
                    .font(.system(size: 13, weight: .medium, design: .default))
                    .foregroundColor(copied ? .grokText : .grokDim)
            }
            .buttonStyle(PlainButtonStyle())
            .help("Copy")

            Button(action: {
                onRegenerate?()
            }) {
                Image(systemName: "arrow.clockwise")
                    .font(.system(size: 13, weight: .medium, design: .default))
                    .foregroundColor(.grokDim)
            }
            .buttonStyle(PlainButtonStyle())
            .help("Regenerate")

            Spacer()

            Button(action: {
                liked = liked == true ? nil : true
                onFeedback?(true)
            }) {
                Image(systemName: liked == true ? "hand.thumbsup.fill" : "hand.thumbsup")
                    .font(.system(size: 13, weight: .medium, design: .default))
                    .foregroundColor(liked == true ? .grokText : .grokDim)
            }
            .buttonStyle(PlainButtonStyle())
            .help("Good response")

            Button(action: {
                liked = liked == false ? nil : false
                onFeedback?(false)
            }) {
                Image(systemName: liked == false ? "hand.thumbsdown.fill" : "hand.thumbsdown")
                    .font(.system(size: 13, weight: .medium, design: .default))
                    .foregroundColor(liked == false ? .grokText : .grokDim)
            }
            .buttonStyle(PlainButtonStyle())
            .help("Bad response")
        }
        .padding(.top, 6)
    }
}
