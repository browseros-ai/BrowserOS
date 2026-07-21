import SwiftUI

struct ChatPanelView: View {
    @ObservedObject var viewModel: ChatViewModel
    @StateObject private var browserOSVM = BrowserOSChatViewModel()
    @State private var isNearBottom = true
    @State private var scrollOffset: CGFloat = 0
    @State private var contentHeight: CGFloat = 0
    @State private var isInputFocused = false

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
                        // Local chat messages.
                        // CRITICAL: snapshot the array once. Indexing the live
                        // `viewModel.messages` by an enumerated index crashes
                        // (EXC_BREAKPOINT) when the array mutates mid-render
                        // (streaming append, regenerate, conversation switch) -
                        // the snapshot index then exceeds the shrunk live array.
                        let localMessages = viewModel.messages
                        ForEach(Array(localMessages.enumerated()), id: \.element.id) { index, message in
                            let isFirstInGroup = index == 0 || localMessages[index - 1].role != message.role
                            let isLastInGroup = index == localMessages.count - 1 || localMessages[index + 1].role != message.role

                            MessageBubbleView(
                                message: message,
                                isFirstInGroup: isFirstInGroup,
                                isLastInGroup: isLastInGroup,
                                isConversationIdle: viewModel.state == .idle,
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

                        // Track total content height for scroll math
                        GeometryReader { geo in
                            Color.clear
                                .preference(
                                    key: ScrollContentHeightPreferenceKey.self,
                                    value: geo.frame(in: .named("scrollArea")).maxY
                                )
                        }
                        .frame(height: 0)
                    }
                }
            }
            .coordinateSpace(name: "scrollArea")
            .onPreferenceChange(ScrollOffsetPreferenceKey.self) { offset in
                scrollOffset = offset
            }
            .onPreferenceChange(ScrollContentHeightPreferenceKey.self) { totalHeight in
                contentHeight = totalHeight
                // If scroll offset + viewport height is close to total content height, we're near bottom
                let viewportHeight = scrollOffset.isZero ? totalHeight : abs(scrollOffset)
                isNearBottom = abs(totalHeight - viewportHeight) < 100
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
                ZStack(alignment: .topLeading) {
                    MacTextEditor(
                        text: $viewModel.inputText,
                        isFocused: $isInputFocused,
                        onSubmit: { triggerSend() }
                    )
                    .frame(minHeight: 28, maxHeight: 120)
                    .onAppear {
                        DispatchQueue.main.async {
                            isInputFocused = true
                        }
                    }

                    if viewModel.inputText.isEmpty {
                        Text("Ask anything...")
                            .font(.system(size: NSFont.systemFontSize))
                            .foregroundColor(.grokDim)
                            .padding(.horizontal, 4)
                            .padding(.vertical, 4)
                            .allowsHitTesting(false)
                    }
                }

                Button(action: {
                    NSLog("[ChatPanel] send button clicked")
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
        NSLog("[ChatPanel] triggerSend called, text='\(text.prefix(40))', isEmpty=\(text.isEmpty)")
        guard !text.isEmpty else { return }

        if browserOSVM.isLikelyCommand(text) {
            NSLog("[ChatPanel] routing to BrowserOS command")
            viewModel.inputText = ""
            browserOSVM.sendMessage(text)
        } else {
            NSLog("[ChatPanel] routing to ChatViewModel.sendMessage")
            Task { await viewModel.sendMessage() }
        }
    }
}

// MARK: - MacTextEditor (NSTextView Wrapper)

final class ChatInputTextView: NSTextView {
    var onSubmit: (() -> Void)?

    override func keyDown(with event: NSEvent) {
        // Return/Enter keyCode = 36, NumpadEnter = 76
        if event.keyCode == 36 || event.keyCode == 76 {
            if NSEvent.modifierFlags.contains(.shift) {
                super.keyDown(with: event)
                return
            }
            NSLog("[ChatInput] Return pressed - triggering onSubmit")
            onSubmit?()
            return
        }
        super.keyDown(with: event)
    }
}

struct MacTextEditor: NSViewRepresentable {
    @Binding var text: String
    @Binding var isFocused: Bool
    var onSubmit: () -> Void

    func makeNSView(context: Context) -> NSScrollView {
        let scrollView = NSScrollView()
        scrollView.hasVerticalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.hasHorizontalScroller = false
        scrollView.drawsBackground = false
        scrollView.borderType = .noBorder

        let textView = ChatInputTextView()
        textView.onSubmit = onSubmit
        textView.isRichText = false
        textView.isEditable = true
        textView.isSelectable = true
        textView.isFieldEditor = false
        textView.allowsUndo = true
        textView.drawsBackground = false
        textView.isAutomaticQuoteSubstitutionEnabled = false
        textView.isAutomaticDashSubstitutionEnabled = false
        textView.font = NSFont.systemFont(ofSize: NSFont.systemFontSize)
        textView.textColor = NSColor.white
        textView.insertionPointColor = NSColor.white
        textView.string = text
        textView.delegate = context.coordinator
        textView.autoresizingMask = [.width, .height]

        // Register for WindowManager first-responder hook
        WindowManager.inputFirstResponder = textView

        scrollView.documentView = textView
        return scrollView
    }

    func updateNSView(_ nsView: NSScrollView, context: Context) {
        guard let textView = nsView.documentView as? ChatInputTextView else { return }
        if textView.string != text {
            let selected = textView.selectedRanges
            textView.string = text
            textView.selectedRanges = selected
        }
        if isFocused, let window = textView.window, window.firstResponder != textView {
            window.makeFirstResponder(textView)
        }
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(self)
    }

    class Coordinator: NSObject, NSTextViewDelegate {
        var parent: MacTextEditor

        init(_ parent: MacTextEditor) {
            self.parent = parent
        }

        func textDidChange(_ notification: Notification) {
            guard let textView = notification.object as? NSTextView else { return }
            parent.text = textView.string
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

struct ScrollContentHeightPreferenceKey: PreferenceKey {
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
