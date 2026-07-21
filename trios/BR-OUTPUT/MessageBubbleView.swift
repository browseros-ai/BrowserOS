import SwiftUI

struct MessageBubbleView: View {
    let message: ChatMessage
    let isFirstInGroup: Bool
    let isLastInGroup: Bool
    var isConversationIdle: Bool = true
    var onTaskAction: ((UUID, AgentTaskState) -> Void)?
    var onRegenerate: (() -> Void)?
    var onFeedback: ((Bool) -> Void)?

    @State private var isHovered = false

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            if message.role == .assistant {
                avatarView
            } else {
                Spacer(minLength: 4)
            }

            VStack(alignment: message.role == .user ? .trailing : .leading, spacing: 2) {
                if isFirstInGroup {
                    senderLabel
                }

                if message.role == .assistant {
                    assistantContainer
                } else {
                    userBubble
                }

                if isLastInGroup {
                    timestampView
                }
            }

            if message.role == .user {
                avatarView
            } else {
                Spacer(minLength: 4)
            }
        }
        .padding(.horizontal, 12)
        .padding(.top, isFirstInGroup ? 12 : 2)
        .padding(.bottom, isLastInGroup ? 8 : 2)
    }

    // MARK: - Avatar

    private var avatarView: some View {
        Image(systemName: message.role == .user ? "person.fill" : "cpu")
            .font(.system(size: 12, weight: .medium))
            .foregroundColor(message.role == .user ? .grokText : .grokMuted)
            .frame(width: 24, height: 24)
            .background(
                Circle()
                    .fill(message.role == .user ? Color.grokElevated.opacity(0.6) : Color.grokElevated.opacity(0.3))
            )
    }

    // MARK: - Sender Label

    private var senderLabel: some View {
        Text(message.role == .user ? "You" : "TRIOS Agent")
            .font(.system(size: 11, weight: .medium))
            .foregroundColor(.grokMuted)
            .padding(.bottom, 2)
    }

    // MARK: - Timestamp

    private var timestampView: some View {
        Text(message.timestamp, style: .relative)
            .font(.system(size: 9))
            .foregroundColor(.grokDim)
            .padding(.top, 2)
    }

    // MARK: - User Message

    private var userBubble: some View {
        Group {
            if !message.content.isEmpty {
                RichMessageView(text: message.content, isUser: true)
                    .font(.system(size: 15, weight: .regular, design: .default))
                    .foregroundColor(.grokText)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 10)
                    .background(Color.grokElevated.opacity(0.5))
                    .cornerRadius(14, corners: [.topLeft, .topRight, .bottomLeft])
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
        VStack(alignment: .leading, spacing: 12) {
            // Reasoning header
            if !reasoningSegments.isEmpty && isFirstInGroup {
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

            // Tool calls
            if !message.toolCalls.isEmpty {
                VStack(alignment: .leading, spacing: 8) {
                    ForEach(message.toolCalls) { toolCall in
                        ToolCallCardView(toolCall: toolCall)
                    }
                }
            }

            // Streaming indicator
            if message.isStreaming && message.content.isEmpty && message.segments.isEmpty {
                TypingIndicatorView()
            }

            // Inline action bar - only after the entire assistant turn is fully complete
            if !message.isStreaming && !message.content.isEmpty && isLastInGroup && isConversationIdle {
                MessageActionBar(
                    content: message.content,
                    onRegenerate: onRegenerate,
                    onFeedback: onFeedback
                )
            }

            // Hover copy bar - any completed assistant message (ChatGPT / Claude pattern)
            if !isLastInGroup && !message.isStreaming && !message.content.isEmpty {
                HoverCopyBar(content: message.content)
                    .opacity(isHovered ? 1 : 0)
                    .animation(.easeInOut(duration: 0.15), value: isHovered)
            }
        }
        .onHover { hovered in
            isHovered = hovered
        }
    }

    private var reasoningSegments: [String] {
        message.segments.compactMap {
            if case .reasoning(let text) = $0 { return text }
            return nil
        }
    }

    private var reasoningDuration: String {
        let lines = reasoningSegments.reduce(0) { count, text in
            count + text.components(separatedBy: .newlines).filter { !$0.trimmingCharacters(in: .whitespaces).isEmpty }.count
        }
        return lines <= 1 ? "1s" : "\(lines)s"
    }
}

// MARK: - Corner Radius Extension (macOS compatible)

extension View {
    func cornerRadius(_ radius: CGFloat, corners: RectCorner) -> some View {
        clipShape(RoundedCorner(radius: radius, corners: corners))
    }
}

struct RectCorner: OptionSet {
    let rawValue: Int
    static let topLeft = RectCorner(rawValue: 1 << 0)
    static let topRight = RectCorner(rawValue: 1 << 1)
    static let bottomLeft = RectCorner(rawValue: 1 << 2)
    static let bottomRight = RectCorner(rawValue: 1 << 3)
    static let allCorners: RectCorner = [.topLeft, .topRight, .bottomLeft, .bottomRight]
}

struct RoundedCorner: Shape {
    var radius: CGFloat = .infinity
    var corners: RectCorner = .allCorners

    func path(in rect: CGRect) -> Path {
        var path = Path()
        let w = rect.size.width
        let h = rect.size.height
        let r = min(min(radius, h / 2), w / 2)

        let topLeft: CGFloat = corners.contains(.topLeft) ? r : 0
        let topRight: CGFloat = corners.contains(.topRight) ? r : 0
        let bottomLeft: CGFloat = corners.contains(.bottomLeft) ? r : 0
        let bottomRight: CGFloat = corners.contains(.bottomRight) ? r : 0

        path.move(to: CGPoint(x: w / 2.0, y: 0))
        path.addLine(to: CGPoint(x: w - topRight, y: 0))
        path.addArc(center: CGPoint(x: w - topRight, y: topRight), radius: topRight,
                    startAngle: Angle(degrees: -90), endAngle: Angle(degrees: 0), clockwise: false)
        path.addLine(to: CGPoint(x: w, y: h - bottomRight))
        path.addArc(center: CGPoint(x: w - bottomRight, y: h - bottomRight), radius: bottomRight,
                    startAngle: Angle(degrees: 0), endAngle: Angle(degrees: 90), clockwise: false)
        path.addLine(to: CGPoint(x: bottomLeft, y: h))
        path.addArc(center: CGPoint(x: bottomLeft, y: h - bottomLeft), radius: bottomLeft,
                    startAngle: Angle(degrees: 90), endAngle: Angle(degrees: 180), clockwise: false)
        path.addLine(to: CGPoint(x: 0, y: topLeft))
        path.addArc(center: CGPoint(x: topLeft, y: topLeft), radius: topLeft,
                    startAngle: Angle(degrees: 180), endAngle: Angle(degrees: 270), clockwise: false)
        path.closeSubpath()
        return path
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
                    .font(.system(size: 12, weight: .medium, design: .default))
                    .foregroundColor(copied ? .grokText : .grokDim)
            }
            .buttonStyle(PlainButtonStyle())
            .help("Copy")

            Button(action: {
                onRegenerate?()
            }) {
                Image(systemName: "arrow.clockwise")
                    .font(.system(size: 12, weight: .medium, design: .default))
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
                    .font(.system(size: 12, weight: .medium, design: .default))
                    .foregroundColor(liked == true ? .grokText : .grokDim)
            }
            .buttonStyle(PlainButtonStyle())
            .help("Good response")

            Button(action: {
                liked = liked == false ? nil : false
                onFeedback?(false)
            }) {
                Image(systemName: liked == false ? "hand.thumbsdown.fill" : "hand.thumbsdown")
                    .font(.system(size: 12, weight: .medium, design: .default))
                    .foregroundColor(liked == false ? .grokText : .grokDim)
            }
            .buttonStyle(PlainButtonStyle())
            .help("Bad response")
        }
        .padding(.top, 4)
    }
}

// MARK: - Hover Copy Bar (ChatGPT / Claude pattern)

private struct HoverCopyBar: View {
    let content: String
    @State private var copied = false

    var body: some View {
        HStack(spacing: 12) {
            Button(action: {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(content, forType: .string)
                copied = true
                DispatchQueue.main.asyncAfter(deadline: .now() + 2) {
                    copied = false
                }
            }) {
                Image(systemName: copied ? "checkmark" : "doc.on.doc")
                    .font(.system(size: 11, weight: .medium, design: .default))
                    .foregroundColor(copied ? .grokText : .grokDim)
            }
            .buttonStyle(PlainButtonStyle())
            .help("Copy")

            Spacer()
        }
        .padding(.top, 2)
    }
}
