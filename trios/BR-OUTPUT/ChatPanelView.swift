import SwiftUI

struct ChatPanelView: View {
    @ObservedObject var viewModel: ChatViewModel
    @StateObject private var browserOSVM = BrowserOSChatViewModel()
    @State private var isNearBottom = true
    @State private var scrollOffset: CGFloat = 0
    @State private var contentHeight: CGFloat = 0

    var body: some View {
        VStack(spacing: 0) {
            unifiedMessageArea
            unifiedInputBar
        }
        .background(Color.clear)
        .onAppear {
            browserOSVM.startPageDetection()
        }
        .onDisappear {
            browserOSVM.stopPageDetection()
        }
    }

    // MARK: - Unified Messages / Empty State

    private var unifiedMessageArea: some View {
        ScrollViewReader { proxy in
            ScrollView {
                GeometryReader { geo in
                    Color.clear
                        .preference(key: ScrollOffsetPreferenceKey.self, value: geo.frame(in: .named("scrollArea")).minY)
                }
                .frame(height: 0)

                if viewModel.messages.isEmpty && browserOSVM.messages.isEmpty {
                    emptyStateView
                } else {
                    LazyVStack(spacing: 0) {
                        // Local chat messages
                        ForEach(Array(viewModel.messages.enumerated()), id: \.element.id) { index, message in
                            let isFirstInGroup = index == 0 || viewModel.messages[index - 1].role != message.role
                            let isLastInGroup = index == viewModel.messages.count - 1 || viewModel.messages[index + 1].role != message.role

                            MessageBubbleView(
                                message: message,
                                isFirstInGroup: isFirstInGroup,
                                isLastInGroup: isLastInGroup,
                                onTaskAction: { taskId, state in
                                    Task { await viewModel.updateTaskState(id: taskId, state: state) }
                                },
                                onRegenerate: {
                                    Task { await viewModel.regenerateLastResponse() }
                                },
                                onFeedback: { isPositive in
                                    Task { await viewModel.sendFeedback(messageId: message.id, isPositive: isPositive) }
                                }
                            )
                            .id(message.id)
                        }

                        // BrowserOS messages
                        ForEach(browserOSVM.messages) { message in
                            BrowserOSMessageBubble(message: message)
                                .id(message.id)
                        }

                        // Typing indicators
                        if viewModel.state != .idle {
                            TypingIndicatorView()
                                .id("typing-local")
                        }
                        if browserOSVM.isStreaming {
                            TypingIndicatorView()
                                .id("typing-browseros")
                        }
                    }
                }
            }
            .coordinateSpace(name: "scrollArea")
            .onPreferenceChange(ScrollOffsetPreferenceKey.self) { offset in
                scrollOffset = offset
                let threshold: CGFloat = 100
                isNearBottom = abs(offset + contentHeight) < threshold
            }
            .onChange(of: viewModel.messages.count) {
                if isNearBottom, let last = viewModel.messages.last {
                    withAnimation(.easeOut(duration: 0.2)) {
                        proxy.scrollTo(last.id, anchor: .bottom)
                    }
                }
            }
            .onChange(of: viewModel.messages.last?.content) {
                if isNearBottom, let last = viewModel.messages.last {
                    withAnimation(.easeOut(duration: 0.2)) {
                        proxy.scrollTo(last.id, anchor: .bottom)
                    }
                }
            }
            .onChange(of: browserOSVM.messages.count) {
                if isNearBottom, let last = browserOSVM.messages.last {
                    withAnimation(.easeOut(duration: 0.2)) {
                        proxy.scrollTo(last.id, anchor: .bottom)
                    }
                }
            }
            .background(
                GeometryReader { geo in
                    Color.clear
                        .onAppear { contentHeight = geo.size.height }
                        .onChange(of: geo.size.height) { contentHeight = $0 }
                }
            )
        }
    }

    private var emptyStateView: some View {
        VStack(spacing: 24) {
            Spacer()

            logoView(size: CGSize(width: 52, height: 44))

            Text("TRIOS")
                .font(.system(size: 36, weight: .bold, design: .default))
                .foregroundColor(.grokText)

            Text("How can I help?")
                .font(.system(size: 16, weight: .regular, design: .default))
                .foregroundColor(.grokMuted)

            VStack(spacing: 8) {
                suggestedPromptChip("Open google.com in BrowserOS")
                suggestedPromptChip("Take a screenshot of current page")
                suggestedPromptChip("Run /doctor to check build health")
                suggestedPromptChip("Show Queen status overview")
            }
            .padding(.top, 8)

            Spacer()
        }
        .padding(.vertical, 60)
    }

    private func suggestedPromptChip(_ text: String) -> some View {
        Button(action: {
            viewModel.inputText = text
            triggerSend()
        }) {
            Text(text)
                .font(.system(size: 12))
                .foregroundColor(.grokDim)
                .padding(.horizontal, 14)
                .padding(.vertical, 8)
                .background(Color.grokElevated.opacity(0.5))
                .cornerRadius(16)
        }
        .buttonStyle(.plain)
    }

    // MARK: - Unified Input Bar

    private var unifiedInputBar: some View {
        VStack(spacing: 0) {
            Divider().overlay(Color.grokDivider)
            HStack(spacing: 12) {
                TextField("Ask anything...", text: $viewModel.inputText, axis: .vertical)
                    .textFieldStyle(PlainTextFieldStyle())
                    .font(.system(size: 15, weight: .regular, design: .default))
                    .foregroundColor(.grokText)
                    .lineLimit(1...5)
                    .onSubmit {
                        triggerSend()
                    }

                Button(action: {
                    triggerSend()
                }) {
                    Image(systemName: sendButtonIcon)
                        .font(.system(size: 16, weight: .semibold))
                        .foregroundColor(viewModel.inputText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? .grokDim : .grokText)
                        .frame(width: 32, height: 32)
                        .background(
                            Circle()
                                .fill(viewModel.inputText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? Color.clear : Color.grokElevated)
                        )
                }
                .buttonStyle(PlainButtonStyle())
                .disabled(viewModel.inputText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
        }
        .padding(.bottom, 20)
    }

    private var sendButtonIcon: String {
        let isSending = viewModel.state != .idle || browserOSVM.isStreaming
        return isSending ? "stop.fill" : "arrow.up"
    }

    private func triggerSend() {
        let text = viewModel.inputText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }

        if browserOSVM.isLikelyCommand(text) {
            viewModel.inputText = ""
            browserOSVM.sendMessage(text)
        } else {
            Task { await viewModel.sendMessage() }
        }
    }
}

// MARK: - Scroll Offset Tracking

struct ScrollOffsetPreferenceKey: PreferenceKey {
    static var defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = nextValue()
    }
}

// MARK: - Logo Helper

private func logoView(size: CGSize) -> some View {
    Group {
        if let svgURL = Bundle.main.url(forResource: "logo", withExtension: "svg"),
           let nsImage = NSImage(contentsOf: svgURL) {
            Image(nsImage: nsImage)
                .resizable()
                .renderingMode(.template)
                .aspectRatio(contentMode: .fit)
                .frame(width: size.width, height: size.height)
                .foregroundColor(.grokText)
        } else if let pngURL = Bundle.main.url(forResource: "logo", withExtension: "png"),
                  let nsImage = NSImage(contentsOf: pngURL) {
            Image(nsImage: nsImage)
                .resizable()
                .renderingMode(.template)
                .aspectRatio(contentMode: .fit)
                .frame(width: size.width, height: size.height)
                .foregroundColor(.grokText)
        } else if FileManager.default.fileExists(atPath: ProjectPaths.logoSVG),
                  let nsImage = NSImage(contentsOfFile: ProjectPaths.logoSVG) {
            Image(nsImage: nsImage)
                .resizable()
                .renderingMode(.template)
                .aspectRatio(contentMode: .fit)
                .frame(width: size.width, height: size.height)
                .foregroundColor(.grokText)
        } else if FileManager.default.fileExists(atPath: ProjectPaths.logoPNG) {
            Image(nsImage: NSImage(contentsOfFile: ProjectPaths.logoPNG) ?? NSImage())
                .resizable()
                .renderingMode(.template)
                .aspectRatio(contentMode: .fit)
                .frame(width: size.width, height: size.height)
                .foregroundColor(.grokText)
        }
    }
}

// MARK: - BrowserOS Message Bubble

private struct BrowserOSMessageBubble: View {
    let message: BrowserOSChatMessage

    var body: some View {
        HStack {
            if message.role == .user { Spacer() }
            VStack(alignment: .leading, spacing: 4) {
                RichMessageView(text: message.content, isUser: message.role == .user)
                    .font(.system(size: 14, weight: .regular, design: .default))
                    .padding(12)
                    .background(
                        message.role == .user
                            ? Color.grokElevated.opacity(0.8)
                            : Color.grokSurface.opacity(0.6)
                    )
                    .foregroundColor(.grokText)
                    .cornerRadius(16)
                if !message.toolCalls.isEmpty {
                    ForEach(message.toolCalls, id: \.name) { tool in
                        BrowserOSToolCallCard(tool: tool)
                    }
                }
            }
            if message.role == .assistant || message.role == .system { Spacer() }
        }
    }
}

private struct BrowserOSToolCallCard: View {
    let tool: BrowserOSToolCall
    @State private var isExpanded = false

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Image(systemName: "hammer.fill")
                    .foregroundColor(.grokMuted)
                    .font(.caption)
                Text(tool.name)
                    .font(.system(size: 11, weight: .medium))
                    .foregroundColor(.grokText)
                Spacer()
                Button(action: { isExpanded.toggle() }) {
                    Image(systemName: isExpanded ? "chevron.up" : "chevron.down")
                        .font(.caption2)
                        .foregroundColor(.grokMuted)
                }
                .buttonStyle(.plain)
            }
            if isExpanded, let result = tool.result {
                Text(result)
                    .font(.system(size: 11))
                    .foregroundColor(.grokMuted)
                    .padding(6)
                    .background(Color.grokElevated.opacity(0.4))
                    .cornerRadius(6)
            }
        }
        .padding(8)
        .background(Color.grokSurface.opacity(0.4))
        .cornerRadius(8)
        .overlay(
            RoundedRectangle(cornerRadius: 8)
                .stroke(Color.grokBorder.opacity(0.3), lineWidth: 1)
        )
    }
}

// MARK: - Status Dot

private struct StatusDot: View {
    let isOn: Bool
    let label: String?
    let color: Color

    var body: some View {
        HStack(spacing: 4) {
            Circle()
                .fill(isOn ? color : Color.grokDim)
                .frame(width: 6, height: 6)
            if let label = label {
                Text(label)
                    .font(.system(size: 11, weight: .medium, design: .default))
                    .foregroundColor(.grokMuted)
            }
        }
    }
}
